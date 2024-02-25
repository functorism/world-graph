use anyhow::Context;
use axum::extract::MatchedPath;
use axum::http::Request;
use axum::routing::get;
use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use clap::{Parser, ValueEnum};
use futures::future::join_all;
use ollama_rs::generation::{completion::request::GenerationRequest, options::GenerationOptions};
use ollama_rs::Ollama;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tinytemplate::TinyTemplate;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(version, about = "World Graph", long_about = "World Graph")]
struct Args {
    #[arg(short, long, env, default_value = "db.sqlite")]
    sqlite: String,

    #[arg(short, long, env, default_value = "3000")]
    port: u16,

    #[arg(long, env, default_value = "http://localhost")]
    ollama_host: String,

    #[arg(long, env, default_value = "11434")]
    ollama_port: u16,

    #[arg(long, env, default_value = "neural-chat")]
    ollama_model: String,

    #[arg(long, env, default_value = "0.4")]
    ollama_temperature: f32,

    #[arg(long, env, value_enum, default_value = "simple")]
    strategy: StrategyChoice,

    #[arg(long, env, default_value = "3")]
    samples: u8,

    #[arg(short, long, env, default_value = "info")]
    log_level: Level,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum StrategyChoice {
    Simple,
    Sample,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, sqlx::FromRow, Serialize)]
struct Triple {
    a: String,
    b: String,
    c: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
struct Pair {
    a: String,
    b: String,
}

impl Pair {
    fn canonical(self) -> Pair {
        if self.a < self.b {
            Pair {
                a: self.b,
                b: self.a,
            }
        } else {
            self
        }
    }
}

impl From<Triple> for Pair {
    fn from(t: Triple) -> Self {
        Pair { a: t.a, b: t.b }
    }
}

#[derive(Debug, Clone, Copy)]
enum Strategy {
    Simple,
    Sample(u8),
}

#[derive(Debug, Serialize)]
struct AppError {
    error: String,
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError {
            error: e.to_string(),
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError {
            error: e.to_string(),
        }
    }
}

impl From<tinytemplate::error::Error> for AppError {
    fn from(e: tinytemplate::error::Error) -> Self {
        AppError {
            error: e.to_string(),
        }
    }
}

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError { error: e }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        error!("{}", self.error);
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(&self)).into_response()
    }
}

async fn insert_triple(pool: &SqlitePool, a: &str, b: &str, c: &str) -> anyhow::Result<()> {
    let result = sqlx::query!(
        r#"
        INSERT INTO triple (a, b, c)
        VALUES (?, ?, ?)
        "#,
        a,
        b,
        c
    )
    .execute(pool)
    .await;

    match result {
        Ok(_) => {
            info!("inserted: {} + {} = {}", a, b, c);
            Ok(())
        }
        Err(e) => {
            error!("insert error: {:?}", e);
            return Err(e.into());
        }
    }
}

async fn get_triple(pool: &SqlitePool, a: &str, b: &str) -> anyhow::Result<Triple> {
    let result = sqlx::query_as!(
        Triple,
        r#"
        SELECT a, b, c FROM triple WHERE a = ? AND b = ?
        "#,
        a,
        b
    )
    .fetch_one(pool)
    .await
    .context("Failed to fetch from db")?;

    info!("get: {:?}", result);

    Ok(result)
}

async fn get_triples(pool: &SqlitePool) -> anyhow::Result<Vec<Triple>> {
    let result = sqlx::query_as!(Triple, r#"SELECT a, b, c FROM triple"#)
        .fetch_all(pool)
        .await
        .context("Failed to fetch from db")?;

    Ok(result)
}

async fn find_triples(pool: &SqlitePool, a: &str) -> anyhow::Result<Vec<Triple>> {
    let result = sqlx::query_as!(
        Triple,
        r#"
        SELECT a, b, c FROM triple WHERE a = ? OR b = ? OR c = ?
        "#,
        a,
        a,
        a
    )
    .fetch_all(pool)
    .await
    .context("Failed to fetch from db")?;

    Ok(result)
}

#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
    ollama: Ollama,
    ollama_model: String,
    ollama_temperature: f32,
    strategy: Strategy,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(args.log_level)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("setting default subscriber failed")?;

    info!("Ollama: {}:{}", args.ollama_host, args.ollama_port);

    let ollama = ollama_rs::Ollama::new(args.ollama_host, args.ollama_port);

    info!("Sqlite: {}", args.sqlite);
    let pool = SqlitePool::connect(&args.sqlite)
        .await
        .context(format!("Failed to connect to sqlite {}", args.sqlite))?;

    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.port);

    let listener = tokio::net::TcpListener::bind(socket)
        .await
        .context(format!("Failed to bind to address {}", socket))?;

    let strategy = match args.strategy {
        StrategyChoice::Simple => Strategy::Simple,
        StrategyChoice::Sample => Strategy::Sample(args.samples),
    };

    info!("Strategy: {:?}", strategy);

    let app_state = AppState {
        pool,
        ollama,
        ollama_model: args.ollama_model,
        ollama_temperature: args.ollama_temperature,
        strategy,
    };

    let app = Router::new()
        .nest_service("/", ServeDir::new("public"))
        .route("/wander", post(wander))
        .route("/explore", get(explore))
        .layer(CorsLayer::permissive())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                let matched_path = request
                    .extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str);

                tracing::info_span!(
                    "http_request",
                    method = ?request.method(),
                    matched_path,
                    some_other_field = tracing::field::Empty,
                )
            }),
        )
        .with_state(app_state);

    info!("Listening on {}", socket);

    axum::serve(listener, app)
        .await
        .context("Failed to start server")?;

    Ok(())
}

const UNDEF: &str = "undefined";

// A prompt like this should work fairly well for both base and instruction tuned models.
const PROMPT: &str = r#"
Welcome to the World Graph game!

The core idea of World Graph is to explore relationships.
We do this in an algebraic way. Specifically the addition operation.

When two things are combined with `+` we get a third thing.

For example:
% King + Woman = Queen
% Water + Fire = Steam

Addition is commutative, so the order of the things does not matter.
% King + Woman = Queen
% Woman + King = Queen

Not all combinations are sensible, these are undefined.
% Moss + Karl Marx = undefined
% Nuclear + Lipstick = undefined

Using adjectives or adverbs is generally undesirable:
BAD:
% Sand + Water = Wet Sand
GOOD:
% Sand + Water = Mud
BAD:
% Water + Sea = More Water
GOOD:
% Water + Sea = Ocean

Results never contain prose:
BAD:
% Fire + Water = A hot steam vapour
GOOD:
% Fire + Water = Steam
BAD:
% Knowledge + Power = The ability to control people
GOOD:
% Knowledge + Power = Wisdom

Results never grow nominally:
BAD:
% Planet + Planet = Two Planets
GOOD:
% Planet + Planet = Solar System

Countless interesting combinations are possible, and we are just scratching the surface.
In World Graph, you're only limited by your imagination.

You'll soon realize that the game is not about the result, but the journey to get there.
Exciting relationships will be discovered, and you'll be surprised by the results.

For example, you'll discover intriguing examples like:
{examples}
% {a} + {b} ="#;

#[derive(Serialize)]
struct PromptCtx {
    a: String,
    b: String,
    examples: String,
}

fn prompt(a: &str, b: &str, examples: &str) -> Result<String, AppError> {
    let mut tt = TinyTemplate::new();
    tt.add_template("prompt", PROMPT)?;
    let ctx = PromptCtx {
        a: a.to_string(),
        b: b.to_string(),
        examples: examples.to_string(),
    };
    Ok(tt.render("prompt", &ctx)?)
}

async fn get_examples(state: &AppState, pair: &Pair) -> Result<String, AppError> {
    let tsa = find_triples(&state.pool, &pair.a)
        .await?
        .into_iter()
        .take(5);

    let tsb = find_triples(&state.pool, &pair.b)
        .await?
        .into_iter()
        .take(5);

    let mut merged = tsa.chain(tsb).collect::<Vec<Triple>>();
    merged.dedup();

    Ok(merged
        .into_iter()
        .map(|x| format!("% {} + {} = {}", x.a, x.b, x.c))
        .collect::<Vec<String>>()
        .join("\n"))
}

fn process_result(s: &str) -> String {
    s.trim().to_string()
}

async fn wander(
    State(state): State<AppState>,
    Json(pair): Json<Pair>,
) -> Result<Json<Triple>, AppError> {
    let pair = pair.canonical();

    let r = get_triple(&state.pool, &pair.a, &pair.b).await;

    match r {
        Ok(triple) => Ok(triple.into()),
        Err(_) => conjure(&state, &pair).await,
    }
}

async fn completion(state: &AppState, p: &str) -> Result<String, AppError> {
    let req = GenerationRequest::new(state.ollama_model.to_string(), p.to_string())
        .options(
            GenerationOptions::default()
                .temperature(state.ollama_temperature)
                .stop(vec!["\n".to_string(), "(".to_string()]),
        )
        .template("{{ .Prompt }}".to_string());

    Ok(state
        .ollama
        .generate(req.clone())
        .await
        .map(|r| process_result(&r.response))?)
}

async fn conjure(state: &AppState, pair: &Pair) -> Result<Json<Triple>, AppError> {
    let examples = get_examples(state, pair).await?;

    let p = prompt(&pair.a, &pair.b, &examples)?;

    let c = match state.strategy {
        Strategy::Simple => completion(&state, &p).await?,
        Strategy::Sample(n) => {
            let gens = join_all((1..n).into_iter().map(|_| completion(&state, &p)))
                .await
                .into_iter()
                .filter_map(Result::ok)
                .collect::<Vec<String>>();

            let mut counts = std::collections::HashMap::new();

            for g in gens {
                *counts.entry(g).or_insert(0) += 1;
            }

            counts
                .into_iter()
                .max_by_key(|(_, count)| *count)
                .map(|(c, _)| c)
                .unwrap_or_else(|| {
                    error!("Empty samples!");
                    UNDEF.to_string()
                })
        }
    };

    let _ = insert_triple(&state.pool, &pair.a, &pair.b, &c).await?;

    Ok(Triple {
        a: pair.a.clone(),
        b: pair.b.clone(),
        c,
    }
    .into())
}

async fn explore(State(state): State<AppState>) -> Result<Json<Vec<Triple>>, AppError> {
    Ok(Json(get_triples(&state.pool).await?))
}
