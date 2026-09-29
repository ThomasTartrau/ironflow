//! Shared harness for network tests: a local smart-HTTP git server that
//! requires basic auth, backed by `git http-backend` run as a CGI.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use git2::{BranchType, Commit, Oid, Repository, RepositoryInitOptions, Signature};
use ironflow_core::operation::{NoopSecretResolver, OperationContext};
use ironflow_ops_common::MapSecretResolver;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::task::JoinHandle;

/// Token accepted by [`GitHttpServer::start_default`].
pub const TOKEN: &str = "glpat-test-token-0123456789";

/// Username accepted by [`GitHttpServer::start_default`].
pub const USERNAME: &str = "oauth2";

struct ServerConfig {
    root: PathBuf,
    username: String,
    expected_auth: String,
}

/// A smart-HTTP git server on `127.0.0.1` that answers `401` to any request
/// whose `Authorization` header is not the expected basic credentials.
pub struct GitHttpServer {
    root: TempDir,
    port: u16,
    task: JoinHandle<()>,
}

impl GitHttpServer {
    /// Start a server that only accepts `username:password`.
    pub async fn start(username: &str, password: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let expected_auth = format!(
            "Basic {}",
            STANDARD.encode(format!("{username}:{password}"))
        );
        let config = Arc::new(ServerConfig {
            root: root.path().to_path_buf(),
            username: username.to_string(),
            expected_auth,
        });
        let app = Router::new().fallback(handle).with_state(config);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { root, port, task }
    }

    /// Start a server that accepts [`USERNAME`]:[`TOKEN`].
    pub async fn start_default() -> Self {
        Self::start(USERNAME, TOKEN).await
    }

    /// Directory holding the served bare repositories.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// Clone URL of the repository `name` (no credentials in it).
    pub fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{name}", self.port)
    }

    /// Clone URL of the repository `name` with `user:token` embedded.
    pub fn url_with_userinfo(&self, name: &str, user: &str, token: &str) -> String {
        format!("http://{user}:{token}@127.0.0.1:{}/{name}", self.port)
    }

    /// Create the bare repository `name` on the server with one commit on
    /// `main`, pushed over the local filesystem (no auth involved).
    pub fn seed(&self, name: &str) -> Oid {
        let bare = self.root().join(name);
        init_bare_main(&bare);
        let work = tempfile::tempdir().unwrap();
        let oid = commit_on_main(work.path(), "file.txt", "content");
        let repo = Repository::open(work.path()).unwrap();
        let mut remote = repo.remote("seed", bare.to_str().unwrap()).unwrap();
        remote
            .push(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();
        oid
    }

    /// Create an extra branch `branch` on the served repository `name`.
    pub fn create_branch(&self, name: &str, branch: &str, target: Oid) {
        let repo = Repository::open_bare(self.root().join(name)).unwrap();
        let commit = repo.find_commit(target).unwrap();
        repo.branch(branch, &commit, false).unwrap();
    }

    /// Delete the branch `branch` on the served repository `name`.
    pub fn delete_branch(&self, name: &str, branch: &str) {
        let repo = Repository::open_bare(self.root().join(name)).unwrap();
        repo.find_branch(branch, BranchType::Local)
            .unwrap()
            .delete()
            .unwrap();
    }
}

impl Drop for GitHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(State(config): State<Arc<ServerConfig>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let authorized = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some(config.expected_auth.as_str());
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, "Basic realm=\"git\"")],
        )
            .into_response();
    }

    let body = to_bytes(body, usize::MAX).await.unwrap();
    let header = |name: HeaderName| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let mut child = Command::new("git")
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", &config.root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("REQUEST_METHOD", parts.method.as_str())
        .env("PATH_INFO", parts.uri.path())
        .env("QUERY_STRING", parts.uri.query().unwrap_or(""))
        .env("CONTENT_TYPE", header(CONTENT_TYPE))
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("HTTP_CONTENT_ENCODING", header(CONTENT_ENCODING))
        .env("REMOTE_USER", &config.username)
        .env("REMOTE_ADDR", "127.0.0.1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = tokio::spawn(async move {
        stdin.write_all(&body).await.unwrap();
    });
    let output = child.wait_with_output().await.unwrap();
    writer.await.unwrap();
    cgi_response(&output.stdout)
}

fn cgi_response(stdout: &[u8]) -> Response {
    let split = stdout
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p, p + 4))
        .or_else(|| {
            stdout
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|p| (p, p + 2))
        })
        .expect("CGI output without header terminator");
    let head = String::from_utf8_lossy(&stdout[..split.0]);
    let mut status = StatusCode::OK;
    let mut headers: HashMap<String, String> = HashMap::new();
    for line in head.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("status") {
            let code = value.split_whitespace().next().unwrap();
            status = StatusCode::from_u16(code.parse().unwrap()).unwrap();
        } else {
            headers.insert(name.to_string(), value.to_string());
        }
    }
    let mut response = Response::new(Body::from(stdout[split.1..].to_vec()));
    *response.status_mut() = status;
    for (name, value) in headers {
        response.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(&value).unwrap(),
        );
    }
    response
}

/// Initialize a bare repository whose `HEAD` points to `refs/heads/main`.
pub fn init_bare_main(path: &Path) -> Repository {
    let mut opts = RepositoryInitOptions::new();
    opts.bare(true).initial_head("main");
    Repository::init_opts(path, &opts).unwrap()
}

/// Initialize (or reuse) a repository at `path` on `main` and commit `file`
/// with `content`. Returns the new commit id.
pub fn commit_on_main(path: &Path, file: &str, content: &str) -> Oid {
    let repo = match Repository::open(path) {
        Ok(repo) => repo,
        Err(_) => {
            let mut opts = RepositoryInitOptions::new();
            opts.initial_head("main");
            Repository::init_opts(path, &opts).unwrap()
        }
    };
    fs::write(path.join(file), content).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(file)).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&Commit<'_>> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parents)
        .unwrap()
}

/// A local repository on `main` whose `origin` points to `url`.
pub fn local_with_origin(url: &str) -> TempDir {
    let work = tempfile::tempdir().unwrap();
    commit_on_main(work.path(), "local.txt", "local");
    Repository::open(work.path())
        .unwrap()
        .remote("origin", url)
        .unwrap();
    work
}

/// A clone of a parent repository whose `vendor/sub` submodule is declared
/// with `url` in `.gitmodules` but not checked out yet. Returns the directory
/// holding everything and the path of the clone.
///
/// The parent is built with the submodule taken from the server's filesystem,
/// then `.gitmodules` is pointed at `url`: only the operation under test talks
/// to the HTTP server.
pub fn checkout_with_submodule(server: &GitHttpServer, url: &str) -> (TempDir, PathBuf) {
    let dirs = tempfile::tempdir().unwrap();
    let parent_path = dirs.path().join("parent");
    commit_on_main(&parent_path, "README", "parent");
    let mut parent = Repository::open(&parent_path).unwrap();
    let local_sub = server.root().join("sub.git");
    {
        let mut sub = parent
            .submodule(local_sub.to_str().unwrap(), Path::new("vendor/sub"), true)
            .unwrap();
        sub.clone(None).unwrap();
        sub.add_finalize().unwrap();
    }
    parent.submodule_set_url("vendor/sub", url).unwrap();
    let mut index = parent.index().unwrap();
    index.add_path(Path::new(".gitmodules")).unwrap();
    index.write().unwrap();
    let tree = parent.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let head = parent.head().unwrap().peel_to_commit().unwrap();
    parent
        .commit(Some("HEAD"), &sig, &sig, "add submodule", &tree, &[&head])
        .unwrap();

    let clone_path = dirs.path().join("checkout");
    Repository::clone(parent_path.to_str().unwrap(), &clone_path).unwrap();
    (dirs, clone_path)
}

/// Context whose secret store holds `key = value`.
pub fn ctx_with_secret(key: &str, value: &str) -> OperationContext {
    let mut secrets = HashMap::new();
    secrets.insert(key.to_string(), value.to_string());
    OperationContext::new(Arc::new(MapSecretResolver::new(secrets)))
}

/// Context whose secret store holds `git_token = TOKEN`.
pub fn ctx_with_token() -> OperationContext {
    ctx_with_secret("git_token", TOKEN)
}

/// Context with an empty secret store.
pub fn ctx_without_secret() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}
