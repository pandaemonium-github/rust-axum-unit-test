#![allow(dead_code)]
use axum::{
    async_trait,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use axum_macros::debug_handler;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{net::SocketAddr, sync::Arc};
use tokio::time;

#[cfg(test)]
use mockall::automock;

#[tokio::main]
async fn main() {
    let repo: HeroesRepositoryState = Arc::new(HeroesRepository());

    let app = Router::new()
        .nest("/heroes/", heroes_routes())
        .with_state(repo);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    println!("Listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

fn heroes_routes() -> Router<DynHeroesRepository> {
    Router::new().route("/", get(get_heroes))
}
// Hero is the model we want to store in the database
#[derive(Serialize)]
#[cfg_attr(test, derive(Deserialize, Eq, PartialEq, Debug, Copy, Clone, Default))]
pub struct Hero {
    pub id: &'static str,
    pub name: &'static str,
}

/// Error that may happen during data access
enum DataAccessError {
    NotFound,
    TechnicalError,
    OtherError,
}

#[cfg_attr(test, automock)]
#[async_trait]
trait HeroesRepositoryTrait {
    async fn get_by_name(&self, name: &str) -> Result<Vec<Hero>, DataAccessError>;
}

/// Dummy implementation for our repository
/// In real life, this repository would access a database with persisted heroes.
struct HeroesRepository();

#[async_trait]
impl HeroesRepositoryTrait for HeroesRepository {
    async fn get_by_name(&self, name: &str) -> Result<Vec<Hero>, DataAccessError> {
        const HEROES: [Hero; 2] = [
            Hero {
                id: "1",
                name: "Wonder Woman",
            },
            Hero {
                id: "2",
                name: "Deadpool",
            },
        ];
        //simulate read from db
        time::sleep(Duration::from_millis(100)).await;

        let found_heroes: Vec<Hero> = HEROES
            .into_iter()
            .filter(|hero: &Hero| {
                if let Some(stripped_name) = name.strip_suffix('%') {
                    hero.name.starts_with(stripped_name)
                } else {
                    hero.name == name
                }
            })
            .collect::<Vec<Hero>>();

        if found_heroes.is_empty() {
            Err(DataAccessError::NotFound)
        } else {
            Ok(found_heroes)
        }
    }
}

type HeroesRepositoryState = Arc<HeroesRepository>;

#[derive(Deserialize)]
pub struct GetHeroFilter {
    name: Option<String>,
}

type DynHeroesRepository = Arc<dyn HeroesRepositoryTrait + Send + Sync>;

#[debug_handler]
async fn get_heroes(
    State(repo): State<DynHeroesRepository>,
    filter: Query<GetHeroFilter>,
) -> impl IntoResponse {
    let mut name_filter = filter.name.to_owned().unwrap_or("%".to_string());

    if !name_filter.ends_with('%') {
        name_filter.push('%');
    }

    let result = repo.get_by_name(name_filter.as_str()).await;

    match result {
        Err(DataAccessError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Ok(heroes) => Json(heroes).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::*;
    use axum::{body::Body, http::Request};
    use rstest::rstest;
    use serde_json::Value;
    use tower::ServiceExt;

    fn send_get_request(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .method("GET")
            .body(Body::empty())
            .unwrap()
    }
    #[rstest]
    #[case("/?name=Wonder", "Wonder%", )] // verify that % is appended to the filter
    #[case("/?name=Wonder%", "Wonder%")] // verify that % is not appended to the filter if it already ends with %
    #[case("/", "%")] // verify that % is used as the default filter
    #[tokio::test]
    async fn get_by_name_success(#[case] uri: &'static str, #[case] expected_filter: &'static str) {
        let dummy_heroes = vec![Default::default()];

        let mut repo_mock = MockHeroesRepositoryTrait::new();
        let result = Ok(dummy_heroes.clone());

        repo_mock
            .expect_get_by_name()
            .with(eq(expected_filter))
            .return_once(move |_| result);

        let repo = Arc::new(repo_mock) as DynHeroesRepository;

        let app = heroes_routes().with_state(repo);

        let response = app.oneshot(send_get_request(uri)).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();

        assert!(matches!(body, Value::Array{ .. })); // don't care what the result is: just check we receive json array
    }

    #[rstest]
    #[case(DataAccessError::NotFound, StatusCode::NOT_FOUND)]
    #[case(DataAccessError::TechnicalError, StatusCode::INTERNAL_SERVER_ERROR)]
    #[tokio::test]
    async fn get_by_name_failure(#[case] db_result: DataAccessError, #[case] expected_status: StatusCode) {

        let mut repo_mock = MockHeroesRepositoryTrait::new();

        repo_mock
            .expect_get_by_name()
            .with(eq("Spider%"))
            .return_once(move |_| Err(db_result));

        let repo = Arc::new(repo_mock) as DynHeroesRepository;

        let app = heroes_routes().with_state(repo);

        let response = app
            .oneshot(send_get_request("/?name=Spider"))
            .await
            .unwrap();

        assert_eq!(response.status(), expected_status);
    }
}
