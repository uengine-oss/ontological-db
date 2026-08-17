//! `genai.vector.encode()` — turning text into a vector, inside the database.
//!
//! Vector search takes a vector, and the thing asking the question has a
//! string. Something has to bridge that, and where it sits decides what a
//! client needs to be: if the bridge is client-side, every client needs an
//! embedding model and they must all agree on it; if it is here, a question can
//! be asked in one Cypher statement and the stored vectors and the query vector
//! are produced by the same configuration by construction.
//!
//! Neo4j reached the same conclusion and added `genai.vector.encode`. This is
//! that function, under that name.
//!
//! ## It makes an outbound request from a backend, and that is a real cost
//!
//! A PostgreSQL backend blocked on someone else's HTTP server is a backend not
//! answering queries, and a query that reaches the network is a query whose
//! latency and failure modes are not the database's. That is not hidden behind
//! a default:
//!
//! - **Off unless turned on.** `genai.enabled` must be `on`.
//! - **The endpoint is configuration, never an argument.** Neo4j lets the call
//!   name its own endpoint. Here it cannot: the URL comes from
//!   `genai.endpoint`, so a caller who can write Cypher cannot make the server
//!   fetch a URL of their choosing. Query rights are not fetch rights.
//! - **Bounded.** `genai.timeout_ms` caps the wait; the default is short.
//!
//! Configure it in `og_catalog.setting`:
//!
//! ```sql
//! SELECT og_set_setting('genai.enabled',  'on');
//! SELECT og_set_setting('genai.endpoint', 'http://localhost:11434/api/embed');
//! SELECT og_set_setting('genai.provider', 'ollama');
//! SELECT og_set_setting('genai.model',    'qwen3-embedding:latest');
//! SELECT og_set_setting('genai.dimensions', '1024');
//! ```

use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Value};

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

fn setting(key: &str) -> Option<String> {
    crate::spiu::one::<String>(
        "SELECT value FROM og_catalog.setting WHERE key = $1",
        &[key.into()],
    )
    .ok()
    .flatten()
    .filter(|s| !s.trim().is_empty())
}

/// Write one setting. Exists so configuring this does not mean knowing the
/// catalog's table layout.
#[pg_extern]
fn og_set_setting(key: &str, value: &str) {
    Spi::run_with_args(
        "INSERT INTO og_catalog.setting (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        &[key.into(), value.into()],
    )
    .unwrap_or_else(|e| error!("failed to set '{key}': {e}"));
}

/// The request body each provider expects, and where the vector is in its reply.
///
/// Two shapes cover what people actually run: Ollama's, and the one every
/// OpenAI-compatible server copied. A provider that is neither is refused by
/// name rather than sent a body it will not understand.
fn request_body(provider: &str, model: &str, text: &str) -> Value {
    match provider {
        "ollama" => json!({ "model": model, "input": text }),
        _ => json!({ "model": model, "input": text }),
    }
}

fn extract(provider: &str, reply: &Value) -> Option<Vec<f64>> {
    let list = match provider {
        // {"embeddings": [[...]]}
        "ollama" => reply.get("embeddings")?.get(0)?,
        // {"data": [{"embedding": [...]}]}
        _ => reply.get("data")?.get(0)?.get("embedding")?,
    };
    list.as_array()?
        .iter()
        .map(|v| v.as_f64())
        .collect::<Option<Vec<f64>>>()
}

/// `genai.vector.encode(resource, provider, configuration) → LIST<FLOAT>`
///
/// `configuration` may carry `model` and `dimensions`; it may **not** carry an
/// endpoint or a token — see the module comment. Unknown keys are ignored, as
/// they are in Neo4j.
#[pg_extern]
fn og_genai_encode(
    resource: &str,
    provider: default!(Option<&str>, "NULL"),
    configuration: default!(JsonB, "'{}'"),
) -> Vec<f64> {
    if setting("genai.enabled").as_deref() != Some("on") {
        error!(
            "genai.vector.encode is disabled. It makes an outbound HTTP request from the \
             database, so it is off until that is chosen deliberately: \
             SELECT og_set_setting('genai.enabled', 'on')"
        );
    }
    let endpoint = setting("genai.endpoint").unwrap_or_else(|| {
        error!(
            "no embedding endpoint is configured. The URL is deliberately not an argument — \
             set it with og_set_setting('genai.endpoint', '…')"
        )
    });

    let cfg = &configuration.0;
    let provider = provider
        .map(str::to_ascii_lowercase)
        .or_else(|| setting("genai.provider"))
        .unwrap_or_else(|| "ollama".into())
        .to_ascii_lowercase();
    if !matches!(provider.as_str(), "ollama" | "openai" | "azureopenai") {
        error!(
            "provider '{provider}' is not supported. supported: Ollama, OpenAI, AzureOpenAI \
             (anything speaking the OpenAI /v1/embeddings shape)"
        );
    }

    let model = cfg
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| setting("genai.model"))
        .unwrap_or_else(|| error!("no embedding model configured; set genai.model"));

    let timeout = setting("genai.timeout_ms")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS);

    let mut request = ureq::post(&endpoint).timeout(std::time::Duration::from_millis(timeout));
    if let Some(token) = setting("genai.token") {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }

    let reply: Value = match request.send_json(request_body(&provider, &model, resource)) {
        Ok(response) => response
            .into_json()
            .unwrap_or_else(|e| error!("embedding endpoint returned a body that is not JSON: {e}")),
        Err(e) => error!("embedding request to '{endpoint}' failed: {e}"),
    };

    let mut vector = extract(&provider, &reply).unwrap_or_else(|| {
        error!("embedding endpoint returned no vector in the shape '{provider}' produces")
    });

    // Truncation, then re-normalisation. A Matryoshka-trained model's prefix is
    // a valid smaller embedding, which is what makes this legitimate rather
    // than lossy — and pgvector's HNSW index stops at 2000 dimensions, which is
    // what makes it necessary for a 4096-dimension model. Normalising after the
    // cut is what keeps cosine distance meaningful.
    let dims = cfg
        .get("dimensions")
        .and_then(Value::as_i64)
        .or_else(|| setting("genai.dimensions").and_then(|s| s.parse().ok()));
    if let Some(dims) = dims {
        let dims = dims.max(1) as usize;
        if dims < vector.len() {
            vector.truncate(dims);
        }
    }
    let norm = vector.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        for x in &mut vector {
            *x /= norm;
        }
    }
    vector
}
