#!/usr/bin/env bash
# Build a made-up fleet of real git repositories, for screenshots, recordings
# and demos, in a home folder of its own.
#
# Why not `cohors demo`? That mode is sample data with stubbed actions: it
# says "(demo)" where the roots go and nothing it shows is real. This is the
# opposite: twenty-four real repositories of one product team (Northwind's
# shop: Rust services, a Go service, TypeScript frontends, infra), under a
# fake home's ~/Developer, each a working project with a believable file tree,
# weeks of history by five teammates, and a remote it pushes to. The repos sit
# in the states a working developer's repos get into (unpushed commits,
# stashes, a diverged release branch, repos behind their remote, a stale
# fetch). cohors reads them like any others, and fetch, pull and push
# genuinely work, so nothing on camera is staged. None of it touches, reads or
# names anything in your real home folder.
#
#   scripts/demo-fleet.sh [DIR]        (default: /tmp/cohors-demo)
#
# Then run anything against it with the fake home:
#
#   HOME=/tmp/cohors-demo/home cohors
#
# Remotes read git@gitlab.com:northwind/<repo>.git; the fake home's git config
# rewrites that to a bare repository under DIR/remotes, so no network is used.
#
# The fake home also gets stand-ins for cargo, npm, npx, pnpm, go and helm in
# ~/.local/bin (a recording machine need not have every toolchain): they print
# what a passing build, test or lint prints, read only the project, and write
# nothing outside it.
#
# Re-running rebuilds it from scratch, so every take starts identical.

set -euo pipefail

DEMO="${1:-/tmp/cohors-demo}"
case "$DEMO" in
  /tmp/*|/private/tmp/*|"${TMPDIR:-/nonexistent}"*) ;;
  *) echo "refusing to build a demo fleet outside a temp folder: $DEMO" >&2; exit 1 ;;
esac

rm -rf "$DEMO"
mkdir -p "$DEMO/home/Developer" "$DEMO/remotes" "$DEMO/home/.config/cohors" "$DEMO/home/.cohors" "$DEMO/home/.local/bin"
# The real path (on macOS /tmp is a link to /private/tmp), so the paths git
# reports sit under this HOME and every tool shows them as ~/Developer/...
DEMO="$(cd "$DEMO" && pwd -P)"
export HOME="$DEMO/home"
export GIT_CONFIG_NOSYSTEM=1
DEV="$HOME/Developer"
NOW=$(date +%s)
STUDIO="$(cd "$(dirname "$0")" && pwd)/studio"

# The developer whose laptop this is, and the remote host the team uses.
git config --global user.name "Sam Rivera"
git config --global user.email "sam.rivera@example.com"
git config --global init.defaultBranch main
git config --global advice.detachedHead false
git config --global push.autoSetupRemote true
git config --global url."$DEMO/remotes/".insteadOf "git@gitlab.com:northwind/"
REMOTE="git@gitlab.com:northwind"

# ── the team ────────────────────────────────────────────────────────────────
who() {
  case "$1" in
    sam)    echo "Sam Rivera|sam.rivera@example.com" ;;
    priya)  echo "Priya Nair|priya.nair@example.com" ;;
    marcus) echo "Marcus Chen|marcus.chen@example.com" ;;
    lena)   echo "Lena Okafor|lena.okafor@example.com" ;;
    diego)  echo "Diego Alvarez|diego.alvarez@example.com" ;;
    *) echo "unknown teammate $1" >&2; exit 1 ;;
  esac
}

# Commit everything in the worktree as WHO, DAYS ago. Commits in one repo
# always move forward in time, however the days are jittered.
LAST=0
commit() {
  local days="$1" author="$2" message="$3" when ident
  if [ "$days" = 0 ]; then when=$((NOW - 3600 - RANDOM % 9000)); else when=$((NOW - days * 86400 - RANDOM % 25000)); fi
  if [ "$when" -le "$LAST" ]; then when=$((LAST + 900 + RANDOM % 2400)); fi
  if [ "$when" -ge "$NOW" ]; then when=$((NOW - 600)); fi
  LAST=$when
  ident="$(who "$author")"
  git add -A
  GIT_AUTHOR_NAME="${ident%%|*}" GIT_AUTHOR_EMAIL="${ident#*|}" GIT_AUTHOR_DATE="@$when" \
  GIT_COMMITTER_NAME="${ident%%|*}" GIT_COMMITTER_EMAIL="${ident#*|}" GIT_COMMITTER_DATE="@$when" \
    git commit -q -m "$message"
}

# Write a file (from stdin), making its folder.
put() { mkdir -p "$(dirname "$1")"; cat > "$1"; }
# Append to a file (from stdin).
add() { mkdir -p "$(dirname "$1")"; cat >> "$1"; }

# Words of a commit subject ("feat(api): partial cancellations") in the forms
# code needs: Title, snake, camel, Pascal, kebab.
subject() { local s="${1#*: }"; echo "$s"; }
title() { local s; s="$(subject "$1")"; echo "$(echo "${s:0:1}" | tr '[:lower:]' '[:upper:]')${s:1}"; }
snake() { subject "$1" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/_/g; s/^_+|_+$//g'; }
kebab() { snake "$1" | tr '_' '-'; }
pascal() { snake "$1" | awk -F_ '{for (i = 1; i <= NF; i++) printf "%s%s", toupper(substr($i, 1, 1)), substr($i, 2)}'; }
camel() { local p; p="$(pascal "$1")"; echo "$(echo "${p:0:1}" | tr '[:upper:]' '[:lower:]')${p:1}"; }

# Today's date as a changelog heading would show it.
changelog() {  # changelog "Added|Fixed|Changed" "line"
  add CHANGELOG.md <<EOF
- $2
EOF
}

# ── project skeletons, one per kind ─────────────────────────────────────────
# Each writes the files of a fresh project in the current directory. KIND is
# remembered so `work` knows where a feature or a test goes.

rust_service() {  # rust_service NAME "description" PORT module...
  local name="$1" desc="$2" port="$3"; shift 3
  local crate="${name//-/_}" m
  KIND=rust; FEATURE_FILE=src/routes.rs; TEST_FILE=tests/api.rs; MANIFEST=Cargo.toml
  put Cargo.toml <<EOF
[package]
name = "$name"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
anyhow = "1"
axum = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
EOF
  put .gitignore <<'EOF'
/target
.env
EOF
  put rust-toolchain.toml <<'EOF'
[toolchain]
channel = "1.82"
components = ["rustfmt", "clippy"]
EOF
  put src/main.rs <<EOF
use ${crate}::{config::Config, routes};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let config = Config::from_env()?;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    tracing::info!(port = config.port, "$name listening");
    axum::serve(listener, routes::router(config)).await?;
    Ok(())
}
EOF
  {
    echo "//! $desc"
    echo
    echo "pub mod config;"
    echo "pub mod error;"
    for m in "$@"; do echo "pub mod $m;"; done
    echo "pub mod routes;"
  } > src/lib.rs
  put src/config.rs <<EOF
/// Runtime settings, read from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub port: u16,
    pub database_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "$name".to_string(),
            port: $port,
            database_url: "postgres://localhost/${crate}".to_string(),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self, crate::error::Error> {
        let mut config = Self::default();
        if let Ok(port) = std::env::var("PORT") {
            config.port = port.parse().map_err(|_| crate::error::Error::Config("PORT"))?;
        }
        if let Ok(url) = std::env::var("DATABASE_URL") {
            config.database_url = url;
        }
        Ok(config)
    }
}
EOF
  put src/error.rs <<'EOF'
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bad configuration: {0}")]
    Config(&'static str),
    #[error("not found")]
    NotFound,
    #[error("upstream failed: {0}")]
    Upstream(String),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Error::NotFound => StatusCode::NOT_FOUND,
            Error::Config(_) | Error::Upstream(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
EOF
  put src/routes.rs <<'EOF'
use axum::extract::State;
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

use crate::config::Config;
use crate::error::Error;

pub fn router(config: Config) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .with_state(config)
}

async fn health(State(config): State<Config>) -> Result<Json<Value>, Error> {
    Ok(Json(json!({ "service": config.name, "status": "ok" })))
}
EOF
  for m in "$@"; do
    local P; P="$(pascal ": $m")"
    put "src/$m.rs" <<EOF
//! $(title ": ${m//_/ }").

/// Settings for this part of the service.
#[derive(Debug, Clone)]
pub struct ${P}Settings {
    pub enabled: bool,
}

impl Default for ${P}Settings {
    fn default() -> Self {
        Self { enabled: true }
    }
}
EOF
  done
  put tests/api.rs <<EOF
use ${crate}::config::Config;

#[test]
fn listens_on_port_${port}_by_default() {
    assert_eq!(Config::default().port, $port);
}
EOF
  put Makefile <<'EOF'
.PHONY: build test lint fmt run

build:
	cargo build --release

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

run:
	cargo run
EOF
  put Dockerfile <<EOF
FROM rust:1.82 AS build
WORKDIR /src
COPY . .
RUN cargo build --release

FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/target/release/$name /usr/local/bin/$name
EXPOSE $port
ENTRYPOINT ["/usr/local/bin/$name"]
EOF
  put README.md <<EOF
# $name

$desc

## Running locally

\`\`\`sh
docker compose up -d postgres
make run            # listens on :$port
\`\`\`

## Tests

\`\`\`sh
make test
make lint
\`\`\`

## Configuration

| Variable       | Default                          |
| -------------- | -------------------------------- |
| \`PORT\`         | \`$port\`                           |
| \`DATABASE_URL\` | \`postgres://localhost/${crate}\` |
EOF
  put CHANGELOG.md <<'EOF'
# Changelog

## Unreleased

EOF
}

rust_cli() {  # rust_cli NAME "description"
  local name="$1" desc="$2" crate="${1//-/_}"
  KIND=rust-cli; FEATURE_FILE=src/commands.rs; TEST_FILE=tests/cli.rs; MANIFEST=Cargo.toml
  put Cargo.toml <<EOF
[package]
name = "$name"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "nw"
path = "src/main.rs"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive", "env"] }
clap_complete = "4"
tracing = "0.1"
tracing-subscriber = "0.3"

[dev-dependencies]
assert_cmd = "2"
EOF
  put .gitignore <<'EOF'
/target
EOF
  put src/main.rs <<EOF
use clap::Parser;
use ${crate}::commands::{run, Args};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    run(Args::parse())
}
EOF
  put src/lib.rs <<EOF
//! $desc

pub mod commands;
pub mod config;
EOF
  put src/config.rs <<'EOF'
/// Where the CLI deploys to, from ~/.config/nw/config.toml or the flags.
#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub registry: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { name: "nw".to_string(), registry: "registry.northwind.dev".to_string() }
    }
}
EOF
  put src/commands.rs <<'EOF'
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "nw", about = "Deploy and operate Northwind services")]
pub struct Args {
    /// staging or production
    #[arg(long, env = "NW_ENV", default_value = "staging")]
    pub env: String,
    /// The service to act on
    pub service: Option<String>,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    tracing::info!(env = %args.env, service = ?args.service, "nw");
    Ok(())
}
EOF
  put tests/cli.rs <<'EOF'
use deploy_cli::config::Config;

#[test]
fn defaults_to_the_team_registry() {
    assert_eq!(Config::default().registry, "registry.northwind.dev");
}
EOF
  put Makefile <<'EOF'
.PHONY: build test lint install

build:
	cargo build --release

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

install:
	cargo install --path .
EOF
  put README.md <<EOF
# $name

$desc

\`\`\`sh
make install
nw deploy payment-service --env staging
nw logs order-service --env production
\`\`\`
EOF
  put CHANGELOG.md <<'EOF'
# Changelog

## Unreleased

EOF
}

go_service() {  # go_service NAME "description" PORT
  local name="$1" desc="$2" port="$3"
  KIND=go; FEATURE_FILE=internal/query/query.go; TEST_FILE=internal/query/query_test.go; MANIFEST=go.mod
  put go.mod <<EOF
module gitlab.com/northwind/$name

go 1.23

require (
	github.com/go-chi/chi/v5 v5.1.0
	github.com/blevesearch/bleve/v2 v2.4.3
)
EOF
  put .gitignore <<'EOF'
/bin
.env
EOF
  put cmd/$name/main.go <<EOF
package main

import (
	"log/slog"
	"net/http"
	"os"

	"gitlab.com/northwind/$name/internal/api"
)

func main() {
	addr := ":$port"
	if port := os.Getenv("PORT"); port != "" {
		addr = ":" + port
	}
	slog.Info("$name listening", "addr", addr)
	if err := http.ListenAndServe(addr, api.Router()); err != nil {
		slog.Error("server stopped", "err", err)
		os.Exit(1)
	}
}
EOF
  put internal/api/router.go <<EOF
package api

import (
	"encoding/json"
	"net/http"

	"github.com/go-chi/chi/v5"

	"gitlab.com/northwind/$name/internal/query"
)

// Router serves search over the product index.
func Router() http.Handler {
	r := chi.NewRouter()
	r.Get("/healthz", func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK) })
	r.Get("/search", func(w http.ResponseWriter, req *http.Request) {
		q := query.Parse(req.URL.Query().Get("q"))
		_ = json.NewEncoder(w).Encode(q)
	})
	return r
}
EOF
  put internal/query/query.go <<'EOF'
package query

import "strings"

// Query is a parsed search request.
type Query struct {
	Terms []string `json:"terms"`
}

// Parse splits a raw query into lower-case terms.
func Parse(raw string) Query {
	return Query{Terms: strings.Fields(strings.ToLower(raw))}
}
EOF
  put internal/query/query_test.go <<'EOF'
package query

import "testing"

func TestParseLowercasesTerms(t *testing.T) {
	q := Parse("Red Shoes")
	if len(q.Terms) != 2 || q.Terms[0] != "red" {
		t.Fatalf("got %v", q.Terms)
	}
}
EOF
  put Makefile <<EOF
.PHONY: build test lint run

build:
	go build -o bin/$name ./cmd/$name

test:
	go test ./...

lint:
	go vet ./...

run:
	go run ./cmd/$name
EOF
  put README.md <<EOF
# $name

$desc

\`\`\`sh
make run     # listens on :$port
make test
\`\`\`
EOF
  put CHANGELOG.md <<'EOF'
# Changelog

## Unreleased

EOF
}

ts_project() {  # ts_project NAME "description" KIND(node|web|lib|native) [pnpm]
  local name="$1" desc="$2" kind="$3" pm="${4:-npm}"
  KIND="ts-$kind"; MANIFEST=package.json
  local scripts build test="vitest run" lint="eslint ." dev deps devdeps
  case "$kind" in
    node)   FEATURE_FILE=src/routes.ts; TEST_FILE=test/routes.test.ts
            dev="tsx watch src/index.ts"; build="tsc -p ."
            deps='"fastify": "^5.1.0", "zod": "^3.23.8"' ;;
    web)    FEATURE_FILE=src/App.tsx; TEST_FILE=test/App.test.tsx
            dev="vite"; build="tsc -b && vite build"
            deps='"react": "^18.3.1", "react-dom": "^18.3.1"' ;;
    lib)    FEATURE_FILE=src/index.ts; TEST_FILE=test/components.test.tsx
            dev="storybook dev -p 6006"; build="tsc -b && vite build"
            deps='"react": "^18.3.1"' ;;
    native) FEATURE_FILE=src/App.tsx; TEST_FILE=test/App.test.tsx
            dev="expo start"; build="expo export"
            deps='"expo": "~52.0.11", "react": "18.3.1", "react-native": "0.76.3"' ;;
  esac
  devdeps='"@types/node": "^22.9.0", "eslint": "^9.15.0", "typescript": "^5.6.3", "vitest": "^2.1.5"'
  put package.json <<EOF
{
  "name": "$name",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "$desc",
  "scripts": {
    "dev": "$dev",
    "build": "$build",
    "test": "$test",
    "lint": "$lint",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    $deps
  },
  "devDependencies": {
    $devdeps
  }$( [ "$pm" = pnpm ] && printf ',\n  "packageManager": "pnpm@9.14.2"' )
}
EOF
  put tsconfig.json <<'EOF'
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true,
    "resolveJsonModule": true
  },
  "include": ["src", "test"]
}
EOF
  put .gitignore <<'EOF'
node_modules/
dist/
coverage/
.env
EOF
  put eslint.config.js <<'EOF'
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(js.configs.recommended, ...tseslint.configs.recommended, {
  ignores: ["dist/"],
});
EOF
  case "$kind" in
    node)
      put src/index.ts <<EOF
import Fastify from "fastify";
import { routes } from "./routes";

const app = Fastify({ logger: true });
app.register(routes);
app.listen({ port: Number(process.env.PORT ?? 3000), host: "0.0.0.0" });
EOF
      put src/routes.ts <<'EOF'
import type { FastifyInstance } from "fastify";

export async function routes(app: FastifyInstance) {
  app.get("/healthz", async () => ({ status: "ok" }));
}
EOF
      put test/routes.test.ts <<'EOF'
import Fastify from "fastify";
import { describe, expect, it } from "vitest";
import { routes } from "../src/routes";

describe("routes", () => {
  it("answers health checks", async () => {
    const app = Fastify();
    await app.register(routes);
    const res = await app.inject({ method: "GET", url: "/healthz" });
    expect(res.json()).toEqual({ status: "ok" });
  });
});
EOF
      ;;
    web|native)
      put src/App.tsx <<EOF
export function App() {
  return (
    <main className="app">
      <h1>$name</h1>
    </main>
  );
}
EOF
      [ "$kind" = web ] && put src/main.tsx <<'EOF'
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/global.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
EOF
      [ "$kind" = web ] && put src/styles/global.css <<'EOF'
:root {
  --surface: #ffffff;
  --ink: #1f1e1c;
  --accent: #6d4aff;
  font-family: Inter, system-ui, sans-serif;
}

body {
  margin: 0;
  color: var(--ink);
}
EOF
      [ "$kind" = web ] && put index.html <<EOF
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>$name</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
EOF
      [ "$kind" = web ] && put vite.config.ts <<'EOF'
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  test: { environment: "jsdom" },
});
EOF
      [ "$kind" = native ] && put app.json <<EOF
{
  "expo": {
    "name": "Northwind",
    "slug": "$name",
    "version": "1.9.0",
    "ios": { "bundleIdentifier": "com.northwind.shop" },
    "android": { "package": "com.northwind.shop" }
  }
}
EOF
      put test/App.test.tsx <<'EOF'
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "../src/App";

describe("App", () => {
  it("renders the shell", () => {
    render(<App />);
    expect(screen.getByRole("main")).toBeTruthy();
  });
});
EOF
      ;;
    lib)
      put src/index.ts <<'EOF'
export { Button } from "./components/Button";
EOF
      put src/components/Button.tsx <<'EOF'
import type { ButtonHTMLAttributes } from "react";
import "./button.css";

type Props = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "quiet" };

export function Button({ variant = "primary", className = "", ...rest }: Props) {
  return <button className={["nw-button", "nw-button--" + variant, className].join(" ")} {...rest} />;
}
EOF
      put src/components/button.css <<'EOF'
.nw-button {
  font: inherit;
  border-radius: var(--nw-radius-2);
  padding: var(--nw-space-2) var(--nw-space-4);
}

.nw-button--primary {
  background: var(--nw-accent);
  color: var(--nw-on-accent);
}
EOF
      put src/tokens.css <<'EOF'
:root {
  --nw-accent: #6d4aff;
  --nw-on-accent: #ffffff;
  --nw-radius-2: 6px;
  --nw-space-2: 8px;
  --nw-space-4: 16px;
}
EOF
      put test/components.test.tsx <<'EOF'
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Button } from "../src";

describe("Button", () => {
  it("defaults to the primary variant", () => {
    render(<Button>Save</Button>);
    expect(screen.getByRole("button").className).toContain("nw-button--primary");
  });
});
EOF
      ;;
  esac
  local run="npm run"; [ "$pm" = pnpm ] && run="pnpm"
  put README.md <<EOF
# $name

$desc

## Develop

\`\`\`sh
$pm install
$run dev
\`\`\`

## Check

\`\`\`sh
$run test
$run lint
$run typecheck
\`\`\`
EOF
  put CHANGELOG.md <<'EOF'
# Changelog

## Unreleased

EOF
}

# ── one commit of ordinary work, shaped by its subject ──────────────────────
#   work DAYS WHO "type: subject"
# feat adds code where features go, fix and test add a test, perf and
# refactor touch the code, docs the README, chore and build the manifest.
work() {
  local days="$1" author="$2" message="$3" type="${3%%:*}"
  type="${type%%(*}"
  local T S C P K
  T="$(title "$message")"; S="$(snake "$message")"; C="$(camel "$message")"; P="$(pascal "$message")"; K="$(kebab "$message")"
  case "$type" in
    feat)
      case "$KIND" in
        rust) add "$FEATURE_FILE" <<EOF

/// $T.
pub async fn $S(State(config): State<Config>) -> Result<Json<Value>, Error> {
    tracing::debug!(service = %config.name, "$S");
    Ok(Json(json!({ "ok": true })))
}
EOF
          ;;
        rust-cli) add "$FEATURE_FILE" <<EOF

/// $T.
pub fn $S(args: &Args) -> anyhow::Result<()> {
    tracing::info!(env = %args.env, "$S");
    Ok(())
}
EOF
          ;;
        go) add "$FEATURE_FILE" <<EOF

// $P: $(subject "$message").
func $P(q Query) Query {
	return q
}
EOF
          ;;
        ts-node) add "$FEATURE_FILE" <<EOF

/** $T. */
export async function $C(app: FastifyInstance) {
  app.post("/$K", async (req) => ({ accepted: true, body: req.body }));
}
EOF
          ;;
        ts-web|ts-native|ts-lib) put "src/components/$P.tsx" <<EOF
/** $T. */
export function $P() {
  return <section className="$K" aria-label="$T" />;
}
EOF
          ;;
        tf) put "modules/$K/main.tf" <<EOF
# $T.
variable "environment" {
  type = string
}

locals {
  name = "northwind-\${var.environment}-$K"
}
EOF
          ;;
        helm) put "charts/$K/Chart.yaml" <<EOF
apiVersion: v2
name: $K
description: $T
version: 0.1.0
EOF
          ;;
        proto) put "proto/northwind/$S/v1/$S.proto" <<EOF
syntax = "proto3";

package northwind.$S.v1;

// $T.
message ${P}Event {
  string id = 1;
  int64 occurred_at = 2;
}
EOF
          ;;
      esac
      [ -f CHANGELOG.md ] && changelog Added "$T"
      ;;
    fix|test)
      case "$KIND" in
        rust|rust-cli) add "$TEST_FILE" <<EOF

#[test]
fn $S() {
    let config = Config::default();
    assert!(!config.name.is_empty());
}
EOF
          ;;
        go) add "$TEST_FILE" <<EOF

func Test$P(t *testing.T) {
	if q := Parse(""); len(q.Terms) != 0 {
		t.Fatalf("got %v", q.Terms)
	}
}
EOF
          ;;
        ts-node) add "$TEST_FILE" <<EOF

describe("$(subject "$message")", () => {
  it("still answers health checks", async () => {
    const app = Fastify();
    await app.register(routes);
    const res = await app.inject({ method: "GET", url: "/healthz" });
    expect(res.statusCode).toBe(200);
  });
});
EOF
          ;;
        ts-lib) add "$TEST_FILE" <<EOF

describe("$(subject "$message")", () => {
  it("renders a labelled button", () => {
    render(<Button aria-label="Close">×</Button>);
    expect(screen.getByLabelText("Close")).toBeTruthy();
  });
});
EOF
          ;;
        ts-*) add "$TEST_FILE" <<EOF

describe("$(subject "$message")", () => {
  it("renders without errors", () => {
    const { container } = render(<App />);
    expect(container.innerHTML).not.toBe("");
  });
});
EOF
          ;;
        *) add README.md <<EOF

- $T.
EOF
          ;;
      esac
      [ "$type" = fix ] && [ -f CHANGELOG.md ] && changelog Fixed "$T"
      ;;
    perf|refactor|style)
      case "$KIND" in
        rust|rust-cli) add src/config.rs <<EOF

// $T.
EOF
          ;;
        go) add "$FEATURE_FILE" <<EOF

// $T.
EOF
          ;;
        ts-*) add "$FEATURE_FILE" <<EOF

// $T.
EOF
          ;;
        *) add README.md <<EOF

- $T.
EOF
          ;;
      esac
      [ -f CHANGELOG.md ] && changelog Changed "$T"
      ;;
    docs)
      add README.md <<EOF

## $T

See the team wiki for the full runbook; the short version lives here.
EOF
      ;;
    chore|build|ci)
      # "bump X" moves dependency X on a minor version when the manifest has
      # it; any other chore is a release, and the package's version moves on.
      local dep; dep="$(subject "$message" | sed -nE 's/^bump (the )?([a-z0-9_@/-]+).*/\2/p')"
      if [ -n "$dep" ] && [ -f "$MANIFEST" ] && grep -qE "^ *\"?$dep\"? *[=:]" "$MANIFEST"; then
        DEP="$dep" perl -pi -e 'if (!$done && /^\s*"?\Q$ENV{DEP}\E"?\s*[=:]/) { s/(\d+)\.(\d+)/$1.".".($2+1)/e; $done = 1 }' "$MANIFEST"
      else
      case "$MANIFEST" in
        Cargo.toml|package.json)
          local v; v="$(grep -m1 -E '^(version = |  "version": )' "$MANIFEST" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')"
          local next; next="$(echo "$v" | awk -F. '{print $1"."$2+1".0"}')"
          sed -i '' -E "1,/[0-9]+\.[0-9]+\.[0-9]+/ s/$v/$next/" "$MANIFEST" ;;
        *) add README.md <<EOF

- $T.
EOF
          ;;
      esac
      fi
      ;;
  esac
  commit "$days" "$author" "$message"
}

# A new repository: a skeleton committed DAYS ago by WHO, the history given
# as "days:who:type: subject" entries, and a remote it is level with.
#   start NAME DAYS WHO
#   history "days:who:message" ...
#   publish [BRANCH]
start() {
  local name="$1"
  LAST=0
  mkdir -p "$DEV/$name" && cd "$DEV/$name"
  git init -q -b main
}
history() {
  local entry days author
  for entry in "$@"; do
    days="${entry%%:*}"; entry="${entry#*:}"
    author="${entry%%:*}"; entry="${entry#*:}"
    work "$days" "$author" "$entry"
  done
}
publish() {
  local name; name="$(basename "$PWD")"
  git clone -q --bare . "$DEMO/remotes/$name.git"
  git remote add origin "$REMOTE/$name.git"
  git fetch -q origin
  git branch -q --set-upstream-to=origin/main main
}

# Teammates push to BRANCH: each "who:message" becomes a commit on the
# remote, made in a scratch clone, the way their pushes would arrive.
remote_work() {
  local name="$1" branch="$2"; shift 2
  local work="$DEMO/scratch" entry author saved_last=$LAST
  rm -rf "$work"
  git clone -q "$REMOTE/$name.git" "$work"
  ( cd "$work"
    git switch -q "$branch"
    KIND="$KIND" FEATURE_FILE="$FEATURE_FILE" TEST_FILE="$TEST_FILE" MANIFEST="$MANIFEST"
    LAST=$((NOW - 2 * 86400))
    for entry in "$@"; do
      author="${entry%%:*}"; entry="${entry#*:}"
      work 0 "$author" "$entry"
    done
    git push -q origin "$branch" )
  rm -rf "$work"
  LAST=$saved_last
  git fetch -q origin
}

# The launchpad file a repository commits about itself.
launchpad() {  # launchpad DAYS WHO <<'JSON' ... JSON
  mkdir -p .cohors
  cat > .cohors/repo.json
  commit "$1" "$2" "docs: add run commands and open tasks"
}

# ═════════════════════════════════════════════════════════════════════════════
# The three a coding agent works in (scripts/studio/agents.sh, record.sh).
# Clean and pushed here; what the agents leave is what cohors then shows.
# ═════════════════════════════════════════════════════════════════════════════

# ── payment-service ──────────────────────────────────────────────────────────
start payment-service
rust_service payment-service "Charges, refunds and processor webhooks for every Northwind storefront." 8081 money processor retry webhook
commit 84 marcus "feat: charge intents over the processor API"
history "71:marcus:feat: refunds"
put src/money.rs <<'RS'
//! Amounts in minor units, so no float ever touches a price.

/// An amount of money in the currency's smallest unit (cents for USD).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Currency {
    Usd,
    Eur,
    Jpy,
}

impl Currency {
    /// Digits after the decimal point.
    pub fn exponent(self) -> u32 {
        match self {
            Currency::Jpy => 0,
            Currency::Usd | Currency::Eur => 2,
        }
    }
}

impl Money {
    pub fn new(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }

    /// A share of this amount, rounded half to even (banker's rounding).
    pub fn portion(self, numerator: i64, denominator: i64) -> Self {
        let scaled = self.minor * numerator;
        let (q, r) = (scaled / denominator, scaled % denominator);
        let twice = 2 * r.abs();
        let round_up = twice > denominator.abs() || (twice == denominator.abs() && q % 2 != 0);
        Self::new(if round_up { q + scaled.signum() } else { q }, self.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portion_rounds_half_to_even() {
        let total = Money::new(1005, Currency::Usd);
        assert_eq!(total.portion(1, 2).minor, 502);
    }

    #[test]
    fn yen_has_no_minor_unit() {
        assert_eq!(Currency::Jpy.exponent(), 0);
    }
}
RS
commit 63 priya "fix: round refund portions half to even"
put src/webhook.rs <<'RS'
//! Verifying the processor's webhook signatures.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Webhooks older than this are refused, so a captured one can't be replayed.
pub const TOLERANCE: Duration = Duration::from_secs(300);

#[derive(Debug, PartialEq, Eq)]
pub enum Rejected {
    MissingHeader,
    Stale,
    BadSignature,
}

/// The parts of a `Processor-Signature: t=…,v1=…` header.
pub fn parse_header(header: &str) -> Result<(u64, &str), Rejected> {
    let mut ts = None;
    let mut sig = None;
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", v)) => ts = v.parse().ok(),
            Some(("v1", v)) => sig = Some(v),
            _ => {}
        }
    }
    Ok((ts.ok_or(Rejected::MissingHeader)?, sig.ok_or(Rejected::MissingHeader)?))
}

/// Refuse a webhook signed outside the tolerance window.
pub fn check_freshness(signed_at: u64, now: SystemTime) -> Result<(), Rejected> {
    let now = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    if now.abs_diff(signed_at) > TOLERANCE.as_secs() {
        return Err(Rejected::Stale);
    }
    Ok(())
}
RS
commit 52 lena "feat: webhook signatures"
put tests/webhook.rs <<'RS'
use std::time::{Duration, UNIX_EPOCH};

use payment_service::webhook::{check_freshness, parse_header, Rejected};

#[test]
fn parses_timestamp_and_signature() {
    assert_eq!(parse_header("t=1700000000,v1=abc123"), Ok((1_700_000_000, "abc123")));
}

#[test]
fn refuses_a_header_without_a_signature() {
    assert_eq!(parse_header("t=1700000000"), Err(Rejected::MissingHeader));
}

#[test]
fn refuses_stale_webhooks() {
    let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    assert_eq!(check_freshness(1_700_000_000 - 301, now), Err(Rejected::Stale));
    assert_eq!(check_freshness(1_700_000_000 - 30, now), Ok(()));
}
RS
commit 41 lena "test: replay protection for webhooks"
history "33:marcus:docs: local processor sandbox"
put src/processor.rs <<'RS'
//! The payment processor's charge API, behind a trait so tests can fake it.

use crate::money::Money;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargeRequest {
    pub amount: Money,
    pub customer_id: String,
    pub payment_method: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Charge {
    pub id: String,
    pub amount: Money,
}

/// What the processor can answer instead of a charge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// HTTP 429: the processor is rate-limiting this account.
    RateLimited,
    /// Any 5xx from the processor.
    Server(u16),
    /// The card or the request was refused; retrying will not help.
    Declined(String),
}

impl Error {
    /// Maps an HTTP status from the processor to an error, if it is one.
    pub fn from_status(status: u16, body: &str) -> Option<Self> {
        match status {
            200..=299 => None,
            429 => Some(Error::RateLimited),
            500..=599 => Some(Error::Server(status)),
            _ => Some(Error::Declined(body.to_string())),
        }
    }

    pub fn is_server_error(&self) -> bool {
        matches!(self, Error::Server(_))
    }
}

/// Anything that can create a charge: the HTTP client, or a fake in tests.
pub trait Processor {
    fn charge(&self, req: &ChargeRequest) -> Result<Charge, Error>;
}
RS
commit 26 priya "feat: typed processor errors"
put src/retry.rs <<'RS'
//! Retrying charges when the processor has a bad moment.

use crate::processor::{Charge, ChargeRequest, Error, Processor};

/// Retries after the first attempt.
pub const MAX_RETRIES: u32 = 3;

/// Calls the payment processor, retrying on server errors.
pub fn charge_with_retry<P: Processor>(processor: &P, req: &ChargeRequest) -> Result<Charge, Error> {
    let mut retries = 0;
    loop {
        match processor.charge(req) {
            Ok(charge) => return Ok(charge),
            Err(e) if e.is_server_error() && retries < MAX_RETRIES => retries += 1,
            Err(e) => return Err(e),
        }
    }
}
RS
put tests/retry.rs <<'RS'
use std::cell::RefCell;

use payment_service::money::{Currency, Money};
use payment_service::processor::{Charge, ChargeRequest, Error, Processor};
use payment_service::retry::charge_with_retry;

/// A processor that answers from a script, one reply per call.
struct Scripted(RefCell<Vec<Result<Charge, Error>>>);

impl Processor for Scripted {
    fn charge(&self, _: &ChargeRequest) -> Result<Charge, Error> {
        self.0.borrow_mut().remove(0)
    }
}

fn request() -> ChargeRequest {
    ChargeRequest {
        amount: Money::new(4200, Currency::Usd),
        customer_id: "cus_123".into(),
        payment_method: "pm_card".into(),
    }
}

fn charge() -> Charge {
    Charge { id: "ch_1".into(), amount: Money::new(4200, Currency::Usd) }
}

#[test]
fn retries_a_server_error() {
    let p = Scripted(RefCell::new(vec![Err(Error::Server(502)), Ok(charge())]));
    assert_eq!(charge_with_retry(&p, &request()), Ok(charge()));
}

#[test]
fn gives_up_after_three_retries() {
    let p = Scripted(RefCell::new(vec![Err(Error::Server(503)); 4]));
    assert_eq!(charge_with_retry(&p, &request()), Err(Error::Server(503)));
}

#[test]
fn never_retries_a_decline() {
    let p = Scripted(RefCell::new(vec![Err(Error::Declined("card_declined".into())), Ok(charge())]));
    assert!(matches!(charge_with_retry(&p, &request()), Err(Error::Declined(_))));
}
RS
commit 19 marcus "feat: retry charges on 5xx from the processor"
history "12:sam:fix: duplicate charge on retry" "8:priya:refactor: split the processor client" "5:sam:chore: bump axum"
launchpad 4 sam <<'JSON'
{
  "specVersion": 1,
  "name": "payment-service",
  "summary": "Charges, refunds and webhooks for every storefront",
  "group": "backend",
  "flows": [
    { "id": "dev", "label": "Run locally", "kind": "server", "run": "docker compose up -d postgres && make run", "primary": true },
    { "id": "test", "label": "Test", "kind": "test", "run": "make test" }
  ],
  "agents": [],
  "tasks": [
    { "id": "rate-limit-backoff", "title": "Back off on processor rate limits", "status": "doing", "priority": "high" },
    { "id": "idempotency-keys", "title": "Idempotency keys on charge retries", "status": "todo", "priority": "normal" }
  ]
}
JSON
publish

# ── api-docs ─────────────────────────────────────────────────────────────────
start api-docs
ts_project api-docs "The public API reference and guides at docs.northwind.dev." web
commit 79 lena "feat: docs site shell"
put content/pages.json <<'JSON'
[
  { "title": "Authentication", "path": "/guides/authentication", "headings": ["API keys", "OAuth clients", "Rotating keys"] },
  { "title": "Orders API", "path": "/reference/orders", "headings": ["Create an order", "List orders", "Cancel an order"] },
  { "title": "Payments API", "path": "/reference/payments", "headings": ["Charges", "Refunds", "Webhooks"] },
  { "title": "Rate limits", "path": "/guides/rate-limits", "headings": ["Limits per key", "Retry-After", "Backing off"] },
  { "title": "Webhooks", "path": "/guides/webhooks", "headings": ["Verifying signatures", "Retries", "Event types"] },
  { "title": "Pagination", "path": "/guides/pagination", "headings": ["Cursors", "Page size"] }
]
JSON
commit 66 lena "docs: authentication and webhooks guides"
history "51:diego:docs: orders API reference" "38:lena:feat: versioned docs"
put src/components/Link.tsx <<'TSX'
import type { AnchorHTMLAttributes } from "react";

export function Link({ href, children, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement>) {
  const current = typeof window !== "undefined" && window.location.pathname === href;
  return (
    <a href={href} aria-current={current ? "page" : undefined} {...rest}>
      {children}
    </a>
  );
}
TSX
put src/components/Nav.tsx <<'TSX'
import { Link } from "./Link";
import "./nav.css";

export function Nav() {
  return (
    <nav className="top-nav">
      <Link href="/" className="top-nav__brand">
        Northwind API
      </Link>
      <Link href="/guides">Guides</Link>
      <Link href="/reference">Reference</Link>
      <Link href="/changelog">Changelog</Link>
    </nav>
  );
}
TSX
put src/components/nav.css <<'CSS'
.top-nav {
  display: flex;
  align-items: center;
  gap: 24px;
  height: 56px;
  padding: 0 24px;
  border-bottom: 1px solid #e6e4e0;
}

.top-nav__brand {
  font-weight: 700;
  margin-right: auto;
}
CSS
put src/App.tsx <<'TSX'
import { Nav } from "./components/Nav";

export function App() {
  return (
    <>
      <Nav />
      <main className="app" />
    </>
  );
}
TSX
commit 24 diego "feat: top navigation"
put src/search/index.ts <<'TS'
import pages from "../../content/pages.json";

export type Hit = { title: string; path: string };

/** Pages whose title or headings contain every word of the query. */
export function search(query: string): Hit[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  return pages
    .filter((p) => words.every((w) => (p.title + " " + p.headings.join(" ")).toLowerCase().includes(w)))
    .map((p) => ({ title: p.title, path: p.path }));
}
TS
put test/search.test.ts <<'TS'
import { describe, expect, it } from "vitest";
import { search } from "../src/search";

describe("search", () => {
  it("finds a page by a heading", () => {
    expect(search("refunds").map((h) => h.path)).toEqual(["/reference/payments"]);
  });

  it("needs every word to match", () => {
    expect(search("rate retry")).toEqual([{ title: "Rate limits", path: "/guides/rate-limits" }]);
  });

  it("returns nothing for an empty query", () => {
    expect(search("   ")).toEqual([]);
  });
});
TS
commit 9 lena "feat: search index over every page"
history "4:diego:docs: rate limits guide"
launchpad 3 lena <<'JSON'
{
  "specVersion": 1,
  "name": "api-docs",
  "summary": "Public API reference and guides",
  "group": "frontend",
  "flows": [ { "id": "dev", "label": "Docs preview", "kind": "server", "run": "npm run dev", "port": 5173, "primary": true } ],
  "agents": [],
  "tasks": [
    { "id": "nav-search", "title": "Search box in the top navigation", "status": "todo", "priority": "high" }
  ]
}
JSON
publish

# ── web-storefront: on feat/checkout, five behind its remote ────────────────
start web-storefront
ts_project web-storefront "The storefront customers shop on." web pnpm
commit 88 diego "feat: app shell and routing"
history "74:diego:feat: product pages" "61:priya:feat: product search" "47:diego:fix: hydration warning on product pages"
put src/cart/useCart.ts <<'TS'
import { useSyncExternalStore } from "react";

export type CartItem = { id: string; name: string; quantity: number; price: number };

let items: CartItem[] = [];
const listeners = new Set<() => void>();

export function addToCart(item: CartItem) {
  items = [...items, item];
  listeners.forEach((l) => l());
}

export function useCart() {
  const current = useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => items,
  );
  const total = current.reduce((sum, i) => sum + i.price * i.quantity, 0);
  return { items: current, total: (total / 100).toFixed(2) };
}
TS
put src/cart/CartDrawer.tsx <<'TSX'
import { useCart } from "./useCart";
import "./cart.css";

export function CartDrawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { items, total } = useCart();
  if (!open) return null;
  return (
    <aside className="cart-drawer" aria-label="Cart">
      <header>
        <h2>Your cart</h2>
        <button onClick={onClose} aria-label="Close cart">
          ×
        </button>
      </header>
      <ul>
        {items.map((item) => (
          <li key={item.id}>
            {item.name} × {item.quantity}
          </li>
        ))}
      </ul>
      <footer>Total ${total}</footer>
    </aside>
  );
}
TSX
put src/cart/cart.css <<'CSS'
.cart-drawer {
  position: fixed;
  top: 0;
  right: 0;
  height: 100vh;
  width: 380px;
  background: var(--surface);
  box-shadow: -8px 0 24px rgb(0 0 0 / 0.2);
}

.cart-drawer header {
  display: flex;
  justify-content: space-between;
  padding: 16px 20px;
}
CSS
put test/cart.test.tsx <<'TSX'
import { act, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CartDrawer } from "../src/cart/CartDrawer";
import { addToCart } from "../src/cart/useCart";

describe("CartDrawer", () => {
  it("renders nothing while closed", () => {
    const { container } = render(<CartDrawer open={false} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it("lists items and the total", () => {
    act(() => addToCart({ id: "1", name: "Linen shirt", quantity: 2, price: 4500 }));
    render(<CartDrawer open onClose={() => {}} />);
    expect(screen.getByText("Linen shirt × 2")).toBeTruthy();
    expect(screen.getByText("Total $90.00")).toBeTruthy();
  });
});
TSX
commit 33 diego "feat: cart drawer"
history "25:priya:perf: lazy-load the product gallery" "16:diego:feat: checkout form" "6:sam:fix: tax line wrapping on mobile"
launchpad 5 diego <<'JSON'
{
  "specVersion": 1,
  "name": "web-storefront",
  "summary": "The storefront customers shop on",
  "group": "frontend",
  "flows": [ { "id": "dev", "label": "Dev server", "kind": "server", "run": "pnpm dev", "port": 5173, "primary": true } ],
  "agents": [],
  "tasks": [
    { "id": "cart-drawer", "title": "Slide the cart drawer in", "status": "todo", "priority": "high" },
    { "id": "a11y-checkout", "title": "Keyboard order in the checkout form", "status": "blocked", "priority": "normal" }
  ]
}
JSON
publish
git switch -q -c feat/checkout
put src/checkout/totals.ts <<'TS'
import type { CartItem } from "../cart/useCart";

/** Subtotal, tax and total in cents. */
export function totals(items: CartItem[], taxRate: number) {
  const subtotal = items.reduce((sum, i) => sum + i.price * i.quantity, 0);
  const tax = Math.round(subtotal * taxRate);
  return { subtotal, tax, total: subtotal + tax };
}
TS
commit 2 sam "feat: checkout totals"
git push -q -u origin feat/checkout
remote_work web-storefront feat/checkout \
  "priya:feat: shipping options in checkout" "priya:fix: tax rounding on discounted items" \
  "diego:feat: promo code field" "priya:test: checkout totals edge cases" "diego:style: tighten checkout spacing"

# ═════════════════════════════════════════════════════════════════════════════
# The rest of the fleet, in the states a working week leaves them in.
# ═════════════════════════════════════════════════════════════════════════════

# ── analytics-service and design-system: busy repos, all pushed ─────────────
start analytics-service
rust_service analytics-service "Funnels, retention and rollups over the event stream." 8090 funnels retention rollups
commit 86 lena "feat: event schema and ingestion reader"
history "73:lena:feat: funnels" "58:marcus:feat: retention cohorts" "44:lena:perf: columnar rollups" \
  "29:marcus:fix: timezone bucketing for daily rollups" "15:lena:perf: precompute funnel rollups" "4:sam:perf: incremental funnel rollups"
publish

start design-system
ts_project design-system "Northwind's tokens and React components." lib pnpm
commit 90 priya "feat: tokens and button"
history "64:priya:feat: dialog" "51:diego:feat: data table" "37:priya:fix: focus ring contrast" \
  "22:priya:feat: dark mode tokens" "9:diego:chore: bump react types"
put src/density.css <<'CSS'
[data-density="compact"] {
  --nw-space-2: 6px;
  --nw-space-4: 12px;
}

[data-density="comfortable"] {
  --nw-space-2: 10px;
  --nw-space-4: 20px;
}
CSS
echo 'import "./density.css";' >> src/index.ts
commit 1 sam "feat: density tokens"
publish

# ── mobile-app: release/2.0 diverged (four ahead, one behind), work staged ───
start mobile-app
ts_project mobile-app "The Northwind iOS and Android app." native
commit 81 marcus "feat: onboarding flow"
history "66:marcus:feat: push notifications" "49:lena:fix: crash on cold start" "35:marcus:feat: offline cart" "8:marcus:chore: bump expo sdk"
put src/auth/unlock.ts <<'TS'
import * as LocalAuthentication from "expo-local-authentication";

export async function unlock(): Promise<boolean> {
  const result = await LocalAuthentication.authenticateAsync({ promptMessage: "Unlock Northwind" });
  return result.success;
}
TS
commit 7 marcus "feat: face unlock"
publish
git switch -q -c release/2.0 && git push -q -u origin release/2.0
remote_work mobile-app release/2.0 "lena:fix: keychain read when the device is locked"
for m in "feat: biometric unlock on resume" "fix: token refresh race" "test: unlock flows" "docs: release notes for 2.0"; do work 1 sam "$m"; done
echo 'export const UNLOCK_GRACE_SECONDS = 30;' >> src/auth/unlock.ts
put src/auth/biometrics.ts <<'TS'
import * as LocalAuthentication from "expo-local-authentication";

export async function hasBiometrics(): Promise<boolean> {
  return (await LocalAuthentication.hasHardwareAsync()) && (await LocalAuthentication.isEnrolledAsync());
}
TS
put src/auth/keystore.ts <<'TS'
import * as SecureStore from "expo-secure-store";

export const saveRefreshToken = (token: string) =>
  SecureStore.setItemAsync("refresh-token", token, { keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK });
TS
git add src/auth/unlock.ts src/auth/biometrics.ts src/auth/keystore.ts
sed -i '' 's/^The Northwind iOS and Android app.$/The Northwind iOS and Android app. Release 2.0 adds biometric unlock./' README.md

# ── event-ingest: three behind ───────────────────────────────────────────────
start event-ingest
rust_service event-ingest "Ingests storefront events, backfills and late arrivals." 8085 ingest backfill partitions
commit 83 marcus "feat: ingest endpoint"
history "68:marcus:feat: backfill jobs" "53:lena:fix: late events landing in the wrong partition" \
  "31:marcus:feat: partition pruning" "19:lena:fix: backfill job ordering" "10:marcus:chore: tune batch size"
launchpad 6 marcus <<'JSON'
{
  "specVersion": 1,
  "name": "event-ingest",
  "summary": "Ingest, backfill and late events",
  "group": "backend",
  "flows": [ { "id": "test", "label": "Test", "kind": "test", "run": "make test" } ],
  "agents": [],
  "tasks": [ { "id": "late-events", "title": "Reprocess late events without a full backfill", "status": "todo", "priority": "high" } ]
}
JSON
publish
remote_work event-ingest main "lena:feat: dead-letter queue for malformed events" \
  "marcus:fix: drop duplicate events within a batch" "lena:perf: batch partition writes"

# ── status-page: last fetched forty days ago ────────────────────────────────
start status-page
ts_project status-page "status.northwind.dev: component health and incident history." web
commit 95 diego "feat: status page"
history "78:diego:feat: component groups" "62:priya:feat: incident history" "45:diego:fix: uptime rounding"
publish
touch -t "$(date -v-40d +%Y%m%d%H%M)" "$DEV/status-page/.git/FETCH_HEAD"

# ── deploy-cli: one untracked file ───────────────────────────────────────────
start deploy-cli
rust_cli deploy-cli "nw: deploy, roll back and tail logs for any Northwind service."
commit 77 sam "feat: deploy to staging"
history "59:sam:feat: tail service logs" "41:diego:feat: shell completions" "14:sam:docs: shell completion install notes"
publish
put NOTES.md <<'EOF'
- rollback should print the previous image tag before asking
- `nw logs` needs --since
EOF

# ── more of the services a coding agent works in (scripts/studio/record.sh) ─
# and the rest, level with their remotes.
start auth-service
rust_service auth-service "Sign-in, sessions, passkeys and device management." 8082 sessions rate_limit login passkeys devices
commit 92 priya "feat: sessions"
put src/sessions.rs <<'RS'
//! Signed-in sessions.

/// A signed-in user's session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub user: String,
    pub token: String,
}

impl Session {
    pub fn new(user: &str) -> Self {
        let token = format!("sess_{:016x}", fxhash(user));
        Self { user: user.to_string(), token }
    }
}

/// Compares a stored password digest with the one just entered, in constant time.
pub fn password_matches(stored: &str, given: &str) -> bool {
    stored.len() == given.len() && stored.bytes().zip(given.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}
RS
put src/rate_limit.rs <<'RS'
//! A fixed-window rate limiter, keyed by anything hashable: an IP, an account.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

pub struct RateLimiter<K> {
    limit: u32,
    window: Duration,
    hits: HashMap<K, (Instant, u32)>,
}

impl<K: Eq + Hash> RateLimiter<K> {
    /// At most `limit` attempts per key in each `window`.
    pub fn new(limit: u32, window: Duration) -> Self {
        Self { limit, window, hits: HashMap::new() }
    }

    /// Counts one attempt for `key` at `now`; false once the key is over its limit.
    pub fn allow(&mut self, key: K, now: Instant) -> bool {
        let entry = self.hits.entry(key).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.limit
    }
}
RS
put src/login.rs <<'RS'
//! POST /login and POST /password-reset.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::rate_limit::RateLimiter;
use crate::sessions::{password_matches, Session};

#[derive(Debug, PartialEq, Eq)]
pub enum LoginError {
    BadCredentials,
    TooManyAttempts,
}

pub struct Auth {
    /// email -> password digest
    users: HashMap<String, String>,
    reset_limiter: RateLimiter<IpAddr>,
}

impl Auth {
    pub fn new(users: HashMap<String, String>) -> Self {
        Self {
            users,
            // Password resets send an email, so they are limited per IP.
            reset_limiter: RateLimiter::new(3, Duration::from_secs(3600)),
        }
    }

    /// POST /login
    pub fn login(&mut self, ip: IpAddr, email: &str, password: &str, now: Instant) -> Result<Session, LoginError> {
        let _ = (ip, now);
        match self.users.get(email) {
            Some(stored) if password_matches(stored, password) => Ok(Session::new(email)),
            _ => Err(LoginError::BadCredentials),
        }
    }

    /// POST /password-reset: at most 3 an hour per IP.
    pub fn request_password_reset(&mut self, ip: IpAddr, email: &str, now: Instant) -> Result<(), LoginError> {
        if !self.reset_limiter.allow(ip, now) {
            return Err(LoginError::TooManyAttempts);
        }
        tracing::info!(%email, "password reset requested");
        Ok(())
    }
}
RS
put tests/login.rs <<'RS'
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

use auth_service::login::{Auth, LoginError};

fn auth() -> Auth {
    Auth::new(HashMap::from([("ada@example.com".to_string(), "s3cret-digest".to_string())]))
}

const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));

#[test]
fn signs_in_with_the_right_password() {
    let session = auth().login(IP, "ada@example.com", "s3cret-digest", Instant::now()).unwrap();
    assert_eq!(session.user, "ada@example.com");
}

#[test]
fn refuses_a_wrong_password() {
    let result = auth().login(IP, "ada@example.com", "guess", Instant::now());
    assert_eq!(result, Err(LoginError::BadCredentials));
}

#[test]
fn limits_password_resets_per_ip() {
    let mut auth = auth();
    let now = Instant::now();
    for _ in 0..3 {
        assert_eq!(auth.request_password_reset(IP, "ada@example.com", now), Ok(()));
    }
    assert_eq!(auth.request_password_reset(IP, "ada@example.com", now), Err(LoginError::TooManyAttempts));
}
RS
commit 81 priya "feat: login and password reset"
history "76:priya:feat: passkeys" "57:marcus:fix: clock skew on token expiry" "38:priya:feat: device list" \
  "18:priya:chore: bump jsonwebtoken" "11:sam:fix: logout everywhere" "3:priya:test: passkey recovery"
publish

start order-service
rust_service order-service "Order lifecycle, payments, cancellations and history." 8083 events payments cancellations
commit 89 marcus "feat: order lifecycle"
put src/events.rs <<'RS'
//! Order events, published to the message bus when an order changes.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderEvent {
    Placed { order_id: String },
    Paid { order_id: String, payment_id: String },
    Cancelled { order_id: String },
}

/// Where events go: the bus in production, a Vec in tests.
pub trait Publisher {
    fn publish(&mut self, event: OrderEvent);
}

impl Publisher for Vec<OrderEvent> {
    fn publish(&mut self, event: OrderEvent) {
        self.push(event);
    }
}
RS
put src/payments.rs <<'RS'
//! Reacting to the payment-service's "payment succeeded" webhooks.

use crate::events::{OrderEvent, Publisher};

/// The payment-service retries a webhook until it gets a 2xx, so the same
/// payment can arrive here more than once.
pub struct PaymentHandler<P: Publisher> {
    publisher: P,
}

impl<P: Publisher> PaymentHandler<P> {
    pub fn new(publisher: P) -> Self {
        Self { publisher }
    }

    /// Marks the order paid and tells everyone downstream.
    pub fn payment_succeeded(&mut self, order_id: &str, payment_id: &str) {
        self.publisher.publish(OrderEvent::Paid {
            order_id: order_id.to_string(),
            payment_id: payment_id.to_string(),
        });
    }

    pub fn into_publisher(self) -> P {
        self.publisher
    }
}
RS
put tests/payments.rs <<'RS'
use order_service::events::OrderEvent;
use order_service::payments::PaymentHandler;

#[test]
fn publishes_paid_for_a_payment() {
    let mut handler = PaymentHandler::new(Vec::new());
    handler.payment_succeeded("ord_1", "pay_1");
    assert_eq!(
        handler.into_publisher(),
        vec![OrderEvent::Paid { order_id: "ord_1".into(), payment_id: "pay_1".into() }]
    );
}

#[test]
fn publishes_each_order_once() {
    let mut handler = PaymentHandler::new(Vec::new());
    handler.payment_succeeded("ord_1", "pay_1");
    handler.payment_succeeded("ord_2", "pay_2");
    assert_eq!(handler.into_publisher().len(), 2);
}
RS
commit 72 marcus "feat: publish paid events from payment webhooks"
history "67:marcus:feat: partial cancellations" "43:diego:fix: order totals with partial refunds" \
  "21:marcus:feat: order history paging" "3:sam:perf: index orders by customer"
publish

start cart-service
rust_service cart-service "Carts, saved items and cart merges on sign-in." 8084 carts saved_items
commit 85 diego "feat: carts"
history "62:diego:feat: saved for later" "40:priya:fix: cart merge on sign in" "16:diego:feat: cart expiry"
publish

start catalog-service
rust_service catalog-service "Products, variants, categories and bulk import." 8086 products categories import
commit 93 lena "feat: products and variants"
history "73:lena:feat: categories" "46:marcus:feat: bulk import" "27:lena:fix: slug collisions" "2:sam:feat: variant images"
publish

start inventory-service
rust_service inventory-service "Stock levels, reservations and warehouse transfers." 8087 stock orders transfers
commit 83 marcus "feat: stock levels"
put src/stock.rs <<'RS'
//! Stock per SKU, and reservations held against it.

use std::collections::HashMap;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub on_hand: u32,
    pub reserved: u32,
}

impl Level {
    pub fn available(&self) -> u32 {
        self.on_hand - self.reserved
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StockError {
    UnknownSku(String),
    Insufficient { sku: String, available: u32, wanted: u32 },
}

#[derive(Debug, Default)]
pub struct Inventory {
    levels: HashMap<String, Level>,
}

impl Inventory {
    pub fn receive(&mut self, sku: &str, quantity: u32) {
        self.levels.entry(sku.to_string()).or_default().on_hand += quantity;
    }

    pub fn level(&self, sku: &str) -> Level {
        self.levels.get(sku).copied().unwrap_or_default()
    }

    /// Holds `quantity` of `sku` for an order, so it can't be sold twice.
    pub fn reserve(&mut self, sku: &str, quantity: u32) -> Result<(), StockError> {
        let level = self.levels.get_mut(sku).ok_or_else(|| StockError::UnknownSku(sku.to_string()))?;
        if level.available() < quantity {
            return Err(StockError::Insufficient { sku: sku.to_string(), available: level.available(), wanted: quantity });
        }
        level.reserved += quantity;
        Ok(())
    }
}
RS
put tests/stock.rs <<'RS'
use inventory_service::stock::{Inventory, StockError};

#[test]
fn reserving_lowers_what_is_available() {
    let mut inv = Inventory::default();
    inv.receive("TEE-BLK-M", 10);
    inv.reserve("TEE-BLK-M", 3).unwrap();
    assert_eq!(inv.level("TEE-BLK-M").available(), 7);
}

#[test]
fn cannot_reserve_more_than_is_available() {
    let mut inv = Inventory::default();
    inv.receive("TEE-BLK-M", 2);
    assert_eq!(
        inv.reserve("TEE-BLK-M", 3),
        Err(StockError::Insufficient { sku: "TEE-BLK-M".into(), available: 2, wanted: 3 })
    );
}
RS
commit 65 marcus "feat: reservations"
put src/orders.rs <<'RS'
//! Order events from the order-service.

use crate::stock::{Inventory, StockError};

pub struct OrderLine {
    pub sku: String,
    pub quantity: u32,
}

pub struct OrderPlaced {
    pub order_id: String,
    pub lines: Vec<OrderLine>,
}

/// Called for every OrderPlaced event.
pub fn on_order_placed(inventory: &mut Inventory, event: &OrderPlaced) -> Result<(), StockError> {
    let _ = inventory;
    tracing::info!(order = %event.order_id, lines = event.lines.len(), "order placed");
    Ok(())
}
RS
put tests/orders.rs <<'RS'
use inventory_service::orders::{on_order_placed, OrderLine, OrderPlaced};
use inventory_service::stock::Inventory;

fn order(sku: &str, quantity: u32) -> OrderPlaced {
    OrderPlaced { order_id: "ord_1".into(), lines: vec![OrderLine { sku: sku.into(), quantity }] }
}

#[test]
fn accepts_an_order_for_stock_we_have() {
    let mut inv = Inventory::default();
    inv.receive("TEE-BLK-M", 5);
    assert!(on_order_placed(&mut inv, &order("TEE-BLK-M", 2)).is_ok());
}
RS
commit 41 lena "feat: listen for order placed events"
history "39:lena:fix: negative stock on refunds" "18:marcus:feat: warehouse transfers"
publish

start pricing-service
rust_service pricing-service "Price lists, discounts and tiered pricing." 8088 price_lists discounts
commit 81 priya "feat: price lists"
history "60:priya:feat: discounts" "37:diego:fix: rounding on percentage discounts" "9:priya:feat: tiered pricing" "4:sam:test: discount stacking"
publish

start shipping-service
rust_service shipping-service "Carriers, delivery estimates and labels." 8089 carriers labels
commit 75 diego "feat: carrier integrations"
history "52:diego:feat: delivery estimates" "30:lena:fix: label PDFs on A6 paper" "8:diego:feat: return labels"
publish

start search-service
go_service search-service "Product search: the index, typo tolerance and synonyms." 8091
commit 78 lena "feat: product index"
history "59:lena:feat: typo tolerance" "34:marcus:perf: cache facet counts"
put internal/query/synonyms.go <<'GO'
package query

// synonyms maps a term to the other words shoppers use for the same thing,
// so either spelling finds the same products.
var synonyms = map[string][]string{
	"sneakers":   {"trainers"},
	"trainers":   {"sneakers"},
	"hoodie":     {"sweatshirt"},
	"sweatshirt": {"hoodie"},
}

// Expand returns the query with each term's synonyms added.
func Expand(q Query) Query {
	out := Query{Terms: append([]string{}, q.Terms...)}
	for _, term := range q.Terms {
		out.Terms = append(out.Terms, synonyms[term]...)
	}
	return out
}
GO
put internal/query/synonyms_test.go <<'GO'
package query

import (
	"reflect"
	"testing"
)

func TestExpandAddsSynonyms(t *testing.T) {
	got := Expand(Parse("white sneakers"))
	want := []string{"white", "sneakers", "trainers"}
	if !reflect.DeepEqual(got.Terms, want) {
		t.Fatalf("got %v, want %v", got.Terms, want)
	}
}

func TestExpandLeavesOtherTermsAlone(t *testing.T) {
	got := Expand(Parse("linen shirt"))
	if len(got.Terms) != 2 {
		t.Fatalf("got %v", got.Terms)
	}
}
GO
commit 12 lena "feat: synonyms"
publish

start notification-service
ts_project notification-service "Email, push and digest notifications." node
commit 70 priya "feat: email channel"
put src/settings.ts <<'TS'
/** What a customer has chosen to receive. */
export type NotificationSettings = {
  email: boolean;
  push: boolean;
  dailyDigest: boolean;
};

export const defaultSettings: NotificationSettings = {
  email: true,
  push: true,
  dailyDigest: false,
};
TS
put src/schedule.ts <<'TS'
import type { NotificationSettings } from "./settings";

export type Channel = "email" | "push";

/** Whether to send a notification on `channel` now, or hold it. */
export function shouldSendNow(settings: NotificationSettings, channel: Channel, now: Date): boolean {
  void now;
  return settings[channel];
}
TS
put test/schedule.test.ts <<'TS'
import { describe, expect, it } from "vitest";
import { shouldSendNow } from "../src/schedule";
import { defaultSettings } from "../src/settings";

describe("shouldSendNow", () => {
  it("sends on a channel the customer has on", () => {
    expect(shouldSendNow(defaultSettings, "email", new Date("2026-11-14T15:00:00"))).toBe(true);
  });

  it("holds a channel the customer turned off", () => {
    expect(shouldSendNow({ ...defaultSettings, push: false }, "push", new Date("2026-11-14T15:00:00"))).toBe(false);
  });
});
TS
commit 51 priya "feat: push channel and per-channel settings"
history "36:diego:feat: daily digests" "23:priya:fix: unsubscribe link" "4:sam:feat: per channel opt outs"
publish

start api-gateway
ts_project api-gateway "The public edge: routing, auth and rate limits per key." node
commit 94 marcus "feat: route table"
history "77:marcus:feat: rate limits per key" "54:priya:fix: forward the request id" \
  "29:marcus:feat: canary routing" "6:marcus:fix: trailing slash redirects" "2:sam:chore: bump fastify"
publish

start feature-flags
ts_project feature-flags "Flags, percentage rollouts and kill switches." node
commit 69 diego "feat: flags"
history "44:diego:feat: percentage rollouts" "21:priya:feat: kill switch" "6:diego:chore: remove the checkout v1 flag"
publish

start admin-dashboard
ts_project admin-dashboard "Back office: orders, refunds and customer support." web
commit 87 lena "feat: admin shell"
put src/orders/types.ts <<'TS'
export type Order = {
  id: string;
  customer: string;
  status: "placed" | "paid" | "shipped" | "refunded";
  totalCents: number;
  placedAt: string;
};
TS
put src/lib/format.ts <<'TS'
export const formatMoney = (cents: number) => "$" + (cents / 100).toFixed(2);

export const formatDate = (iso: string) => new Date(iso).toLocaleDateString("en-US", { dateStyle: "medium" });
TS
put src/lib/download.ts <<'TS'
/** Saves `contents` to the browser's downloads as `filename`. */
export function download(filename: string, contents: string, type = "text/plain") {
  const url = URL.createObjectURL(new Blob([contents], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  URL.revokeObjectURL(url);
}
TS
put src/orders/OrdersTable.tsx <<'TSX'
import { formatDate, formatMoney } from "../lib/format";
import type { Order } from "./types";

export function OrdersTable({ orders }: { orders: Order[] }) {
  return (
    <section className="orders">
      <header className="orders__header">
        <h2>Orders</h2>
      </header>
      <table>
        <thead>
          <tr>
            <th>Order</th>
            <th>Customer</th>
            <th>Status</th>
            <th>Total</th>
            <th>Placed</th>
          </tr>
        </thead>
        <tbody>
          {orders.map((o) => (
            <tr key={o.id}>
              <td>{o.id}</td>
              <td>{o.customer}</td>
              <td>{o.status}</td>
              <td>{formatMoney(o.totalCents)}</td>
              <td>{formatDate(o.placedAt)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
TSX
put test/OrdersTable.test.tsx <<'TSX'
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { OrdersTable } from "../src/orders/OrdersTable";

const orders = [
  { id: "ord_1042", customer: "Ada Lovelace", status: "paid" as const, totalCents: 8900, placedAt: "2026-11-12T10:00:00Z" },
];

describe("OrdersTable", () => {
  it("shows each order with its total", () => {
    render(<OrdersTable orders={orders} />);
    expect(screen.getByText("ord_1042")).toBeTruthy();
    expect(screen.getByText("$89.00")).toBeTruthy();
  });
});
TSX
commit 64 lena "feat: orders table"
history "42:diego:feat: refund approvals" "20:lena:fix: date filters in UTC" "7:sam:feat: customer timeline"
publish

# ── infrastructure ───────────────────────────────────────────────────────────
start infra
KIND=tf; MANIFEST=none
put main.tf <<'EOF'
terraform {
  required_version = ">= 1.9"
  required_providers {
    aws = { source = "hashicorp/aws", version = "~> 5.75" }
  }
  backend "s3" {
    bucket = "northwind-terraform-state"
    key    = "platform/terraform.tfstate"
    region = "eu-west-1"
  }
}

module "network" {
  source      = "./modules/network"
  environment = var.environment
}
EOF
put variables.tf <<'EOF'
variable "environment" {
  description = "staging or production"
  type        = string
}
EOF
put modules/network/main.tf <<'EOF'
variable "environment" {
  type = string
}

resource "aws_vpc" "main" {
  cidr_block = "10.40.0.0/16"
  tags       = { Name = "northwind-${var.environment}" }
}
EOF
put envs/staging.tfvars <<'EOF'
environment = "staging"
EOF
put envs/production.tfvars <<'EOF'
environment = "production"
EOF
put .gitignore <<'EOF'
.terraform/
*.tfstate*
*.tfplan
EOF
put Makefile <<'EOF'
ENV ?= staging

.PHONY: init plan apply fmt

init:
	terraform init

plan:
	terraform plan -var-file=envs/$(ENV).tfvars -out=$(ENV).tfplan

apply:
	terraform apply $(ENV).tfplan

fmt:
	terraform fmt -recursive
EOF
put README.md <<'EOF'
# infra

Terraform for Northwind's AWS accounts: network, clusters and databases.

```sh
make plan ENV=staging
make apply ENV=staging
```
EOF
commit 61 marcus "feat: base network"
history "45:marcus:feat: staging cluster" "27:lena:build: pin the aws provider" "11:marcus:feat: read replicas"
publish

start helm-charts
KIND=helm; MANIFEST=none
put charts/service/Chart.yaml <<'EOF'
apiVersion: v2
name: service
description: The shared chart every Northwind service deploys with
version: 1.3.0
EOF
put charts/service/values.yaml <<'EOF'
image:
  repository: registry.northwind.dev/service
  tag: latest
replicas: 2
resources:
  requests:
    cpu: 100m
    memory: 128Mi
  limits:
    memory: 256Mi
EOF
put charts/service/templates/deployment.yaml <<'EOF'
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ .Release.Name }}
spec:
  replicas: {{ .Values.replicas }}
  selector:
    matchLabels: { app: {{ .Release.Name }} }
  template:
    metadata:
      labels: { app: {{ .Release.Name }} }
    spec:
      containers:
        - name: app
          image: "{{ .Values.image.repository }}:{{ .Values.image.tag }}"
          resources: {{- toYaml .Values.resources | nindent 12 }}
EOF
put Makefile <<'EOF'
SERVICES := $(basename $(notdir $(wildcard values/*.yaml)))

.PHONY: lint template

lint:
	helm lint charts/service $(foreach s,$(SERVICES),-f values/$(s).yaml)

template:
	helm template $(SERVICE) charts/service -f values/$(SERVICE).yaml
EOF
put README.md <<'EOF'
# helm-charts

The shared chart every Northwind service deploys with (`charts/service`), and
each service's values (`values/<service>.yaml`).

```sh
make lint
make template SERVICE=payment-service
```
EOF
commit 80 marcus "feat: base chart"
for svc in payment-service order-service auth-service inventory-service search-service; do
  case "$svc" in payment-service) replicas=3 ;; *) replicas=2 ;; esac
  put "values/$svc.yaml" <<EOF
image:
  repository: registry.northwind.dev/$svc
replicas: $replicas
resources:
  requests:
    cpu: 200m
    memory: 256Mi
  limits:
    memory: 256Mi
EOF
done
commit 57 marcus "feat: per-service values"
put charts/service/templates/hpa.yaml <<'EOF'
{{- if .Values.autoscaling.enabled }}
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: {{ .Release.Name }}
spec:
  scaleTargetRef: { apiVersion: apps/v1, kind: Deployment, name: {{ .Release.Name }} }
  minReplicas: {{ .Values.replicas }}
  maxReplicas: {{ .Values.autoscaling.maxReplicas }}
  metrics:
    - type: Resource
      resource: { name: cpu, target: { type: Utilization, averageUtilization: 70 } }
{{- end }}
EOF
cat >> charts/service/values.yaml <<'EOF'
autoscaling:
  enabled: false
  maxReplicas: 6
EOF
commit 32 lena "feat: horizontal autoscaling"
sed -i '' 's/^version: 1.3.0$/version: 1.4.0/' charts/service/Chart.yaml
commit 7 marcus "chore: bump the service chart to 1.4.0"
publish

start shared-protos
KIND=proto; MANIFEST=none
put buf.yaml <<'EOF'
version: v2
modules:
  - path: proto
lint:
  use: [STANDARD]
breaking:
  use: [FILE]
EOF
put proto/northwind/orders/v1/orders.proto <<'EOF'
syntax = "proto3";

package northwind.orders.v1;

message Order {
  string id = 1;
  string customer_id = 2;
  int64 total_minor = 3;
  string currency = 4;
}
EOF
put Makefile <<'EOF'
.PHONY: lint breaking generate

lint:
	buf lint

breaking:
	buf breaking --against '.git#branch=main'

generate:
	buf generate
EOF
put README.md <<'EOF'
# shared-protos

Protobuf contracts shared by every Northwind service. `make lint` before a PR;
`make breaking` tells you if a change would break a consumer.
EOF
commit 96 lena "feat: order and payment protos"
history "58:lena:feat: inventory events" "33:marcus:feat: shipment events" "1:lena:feat: refund events"
publish

# ── the user's own files: config, and the focus board ────────────────────────
cat > "$HOME/.config/cohors/config.toml" <<'TOML'
roots = ["~/Developer"]
theme = "dark"
TOML
cat > "$HOME/.cohors/board.json" <<JSON
{
  "specVersion": 1,
  "updated": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "focus": [
    { "id": "rate-limit-backoff", "title": "Back off on processor rate limits", "repo": "payment-service", "status": "doing" },
    { "id": "cart-drawer", "title": "Slide the cart drawer in", "repo": "web-storefront", "status": "todo" },
    { "id": "late-events", "title": "Reprocess late events without a full backfill", "repo": "event-ingest", "status": "todo" }
  ]
}
JSON

# A plain prompt for the fake home's shell: where you are, then ❯.
cat > "$HOME/.zshrc" <<'ZSH'
PROMPT='%F{244}%~%f %F{141}❯%f '
ZSH

# ── stand-in toolchains on the fake home's PATH ──────────────────────────────
cp "$STUDIO/toolchain.py" "$HOME/.local/bin/.toolchain"
chmod +x "$HOME/.local/bin/.toolchain"
for tool in cargo npm npx pnpm go gofmt helm rustc rustfmt node; do
  ln -s .toolchain "$HOME/.local/bin/$tool"
done

echo "demo fleet ready: $DEMO ($(ls "$DEV" | wc -l | tr -d ' ') repositories)"
echo "  HOME=$HOME cohors"
