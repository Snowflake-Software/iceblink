pub mod auth;
pub mod cli;
pub mod icons;
pub mod models;
pub mod routes;
pub mod utils;

use axum::extract::{MatchedPath, Request};
use axum::http::{header, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::{middleware, Router};
use icons::IconStore;
use memory_serve::{load_assets, MemoryServe};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use serde::Serialize;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::time::Instant;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::info;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

#[derive(Clone)]
pub struct ServerOptions {
    pub port: u32,
    pub jwt_secret: String,
    pub client_id: String,
    pub client_secret: String,
    pub oauth_server: String,
    pub redirect_uri: String,
    pub frontfacing: String,
}

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub settings: ServerOptions,
    pub openid: auth::OpenId,
    pub icon_store: IconStore,
    pub metrics: PrometheusHandle,
}

#[derive(Debug, Serialize)]
struct ApiDocumentationBearer;
impl Modify for ApiDocumentationBearer {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(schema) = openapi.components.as_mut() {
            schema.add_security_scheme(
                "bearer",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some("Requires a Json Web Token (JWT) for authentication. Generated by the /v1/oauth endpoint"))
                        .build(),
                ),
            );

            schema.add_security_scheme(
                "cookie",
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                    "iceblink_jwt",
                    "Requires a Json Web Token (JWT) for authentication. Generated by the /v1/oauth endpoint",
                ))),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
	tags(
		(name = "codes", description = "Code management endpoints"),
		(name = "user", description = "User endpoints"),
		(name = "misc", description = "Other endpoints")
	),
	servers(
		(url = "http://localhost:8085", description = "Local development server"),
		(url = "https://iceblink.snowflake.blue", description = "Production server")
	),
	info(
		title ="Iceblink Sync Server",
		contact(
			url="https://snowflake.blue",
			name="Snowflake-Software",
		),
		license(
			name="AGPLv3",
			identifier="AGPL-3.0-or-later"
		)
	),
	modifiers(&ApiDocumentationBearer),
	security(
		("bearer" = []),
		("cookie" = []),
	)
)]
struct ApiDocumentation;

#[bon::builder]
pub fn configure_router(pool: &SqlitePool, opts: ServerOptions, openid: auth::OpenId) -> Router {
    let state = Arc::new(AppState {
        db: pool.clone(),
        settings: opts.clone(),
        openid,
        icon_store: IconStore {},
        metrics: setup_metrics_recorder(),
    });

    // Note: Read bottom to top
    let (router, api) = OpenApiRouter::with_openapi(ApiDocumentation::openapi())
        .routes(routes!(
            routes::v1::codes::list_all_codes,
            routes::v1::codes::add_code
        ))
        .routes(routes!(
            routes::v1::codes::delete_code,
            routes::v1::codes::edit_code
        ))
        .routes(routes!(routes::v1::codes::code_icon))
        .routes(routes!(routes::v1::users::delete_account))
        .routes(routes!(routes::v1::users::checksum))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::jwt_middleware,
        ))
        .routes(routes!(routes::v1::misc::instance_metadata))
        .routes(routes!(routes::v1::misc::metrics))
        .routes(routes!(routes::v1::users::oauth))
        .with_state(state)
        .nest_service(
            "/",
            MemoryServe::new(load_assets!("./src/static"))
                .index_file(Some("/landing.html"))
                .html_cache_control(memory_serve::CacheControl::Long)
                .enable_clean_url(true)
                .enable_brotli(true)
                .enable_gzip(true)
                .into_router(),
        )
        .split_for_parts();
    router
        .merge(SwaggerUi::new("/swagger").url("/openapi.json", api))
        .layer(
            CorsLayer::new()
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::PATCH,
                ])
                .allow_origin(
                    opts.frontfacing
                        .parse::<HeaderValue>()
                        .expect("Unable to parse frontfacing URL for CORS"),
                )
                .allow_credentials(true)
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
        )
        .layer(
            CompressionLayer::new()
                .br(true)
                .deflate(true)
                .gzip(true)
                .zstd(true)
                .quality(tower_http::CompressionLevel::Fastest),
        )
        .route_layer(middleware::from_fn(track_metrics))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(Duration::from_secs(2)))
}

pub async fn serve(opts: ServerOptions) {
    info!("Connecting to SQLite: iceblink.db");
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename("iceblink.db")
            .create_if_missing(true),
    )
    .await
    .expect("Unable to connect with SQLite");

    info!("Running SQL migrations");
    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("Unable to run database migrations");

    info!("Discovering OpenId configuration");
    let openid = auth::OpenId::discover()
        .client_id(opts.clone().client_id)
        .client_secret(opts.clone().client_secret)
        .server(opts.clone().oauth_server)
        .call()
        .await
        .expect("Unable to setup OpenId authentication");

    info!("Configuring HTTP router");
    let routes = configure_router()
        .pool(&pool)
        .opts(opts.clone())
        .openid(openid)
        .call();

    info!("Starting HTTP server");
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", opts.port))
        .await
        .unwrap();

    info!("Listening on http://{}", listener.local_addr().unwrap());
    axum::serve(listener, routes)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Exit imminent")
}

fn setup_metrics_recorder() -> PrometheusHandle {
    const EXPONENTIAL_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("http_requests_duration_seconds".to_string()),
            EXPONENTIAL_SECONDS,
        )
        .unwrap()
        .install_recorder()
        .unwrap()
}

async fn track_metrics(req: Request, next: Next) -> impl IntoResponse {
    let start = Instant::now();
    let path = if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
        matched_path.as_str().to_owned()
    } else {
        req.uri().path().to_owned()
    };
    let method = req.method().clone();

    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    let labels = [
        ("method", method.to_string()),
        ("path", path),
        ("status", status),
    ];

    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_requests_duration_seconds", &labels).record(latency);

    response
}
