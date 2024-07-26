use std::{convert::Infallible, future::IntoFuture, net::SocketAddr};

use axum::{
    extract::Request,
    http::StatusCode,
    response::Response,
    routing::{get, post},
    Router, ServiceExt,
};
use lazy_static::lazy_static;
use pulldown_cmark::html;
use tokio::net::TcpListener;

const INDENT_SIZE: usize = 2;

lazy_static! {
    static ref INDEX_HTML_BODY: String = render_md_to_html(include_str!("../README.md"));
    static ref INDEX_HTML: String = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Home</title>
    <link rel="stylesheet" type="text/css" href="https://cdnjs.cloudflare.com/ajax/libs/normalize/8.0.0/normalize.min.css" />
    <style>
    blockquote {{
        display: none;
    }}
    </style>
</head>
<body>
    <div style="max-width: 800px; margin: 0 auto; padding: 20px;">
        {}
        <div>Index generated from README.md</div>
    </div>
</body>
</html>"#,
        *INDEX_HTML_BODY
    );
}

fn render_md_to_html(md: &str) -> String {
    let mut options = pulldown_cmark::Options::empty();
    options.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    let parser = pulldown_cmark::Parser::new_ext(md, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

fn peek_jinja_stmt_keyword(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let kind = node.kind();
    if kind != "statement" {
        return None;
    }
    if let Some(keyword_node) = node.child(1) {
        if let Ok(text) = keyword_node.utf8_text(source) {
            return match text {
                "if" | "elif" | "else" | "endif" | "for" | "endfor" | "macro" | "endmacro"
                | "call" | "endcall" | "filter" | "endfilter" => {
                    return Some(text.to_string());
                }
                _ => None,
            };
        }
    }
    None
}

fn format_jinja_node(root_node: tree_sitter::Node, source: &[u8]) -> String {
    let mut formatted = "".to_string();
    // dfs
    let mut curr_ident = 0;
    let mut next_ident = 0;
    let mut last_node_kind = "";

    for i in 0..root_node.child_count() {
        let node = root_node.child(i).unwrap();
        let keyword = peek_jinja_stmt_keyword(node, source);
        if let Some(keyword) = keyword {
            match keyword.as_str() {
                "if" | "for" | "macro" | "call" | "filter" => {
                    next_ident += 1;
                }
                "elif" | "else" => {
                    curr_ident -= 1;
                }
                "endif" | "endfor" | "endmacro" | "endcall" | "endfilter" => {
                    curr_ident -= 1;
                    next_ident -= 1;
                }
                _ => {
                    panic!("unknown keyword: {}", keyword);
                }
            }
        }

        if node.kind() != "expression" || last_node_kind != "expression" {
            formatted.push('\n');
            formatted.push_str(&" ".repeat(curr_ident * INDENT_SIZE));
        }

        let raw_text = node.utf8_text(source).unwrap();
        formatted.push_str(raw_text);

        last_node_kind = node.kind();
        curr_ident = next_ident;
    }
    formatted[1..].to_string() + "\n"
}

#[derive(serde::Deserialize)]
struct FormatRequestBody {
    input: String,
}

async fn format_jinja(body: String) -> Result<Response, Infallible> {
    let input = serde_json::from_str::<FormatRequestBody>(&body);
    if input.is_err() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "text/plain")
            .body("Invalid request body JSON".to_string().into())
            .unwrap());
    }
    let input = input.unwrap().input;

    let jinja_template = input.as_str();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_jinja2::language())
        .expect("Error loading jinja2 grammar");
    let tree = parser
        .parse(jinja_template, None)
        .expect("Failed to parse code");

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .body(format_jinja_node(tree.root_node(), jinja_template.as_bytes()).into())
        .unwrap())
}

async fn index() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html")
        .body(INDEX_HTML.clone().into())
        .unwrap()
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let router = Router::new()
        .route("/format", post(format_jinja))
        .route("/", get(index));

    let listener = TcpListener::bind("0.0.0.0:18018").await.unwrap();
    println!("Listening on http://0.0.0.0:18018");

    axum::serve(
        listener,
        ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(router),
    )
    .into_future()
    .await
    .unwrap();
}
