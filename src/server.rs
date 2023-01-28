use std::fs::File;
use std::str;

use super::model::*;

use tiny_http::{Server, Request, Response, Header, Method, StatusCode};

fn serve_static_file(request: Request, file_path: &str, content_type: &str) -> Result<(), ()> {
    let content_type_header = Header::from_bytes("Content-Type", content_type)
        .expect("That we didn't put any garbage in the headers");
    // TODO: check if file exists and if it doesn't serve 404
    let file = File::open(file_path).map_err(|err| {
        eprintln!("ERROR: could not serve file {file_path}: {err}");
    })?;
    let response = Response::from_file(file).with_header(content_type_header);
    request.respond(response).map_err(|err| {
        eprintln!("ERROR: could not serve static file {file_path}: {err}");
    })
}

fn serve_404(request: Request) -> Result<(), ()> {
    request.respond(Response::from_string("404").with_status_code(StatusCode(404))).map_err(|err| {
        eprintln!("ERROR: could not serve a request: {err}");
    })
}

fn serve_api_search(tf_index: &TermFreqIndex, mut request: Request) -> Result<(), ()> {
    let mut buf = Vec::new();
    request.as_reader().read_to_end(&mut buf).map_err(|err| {
        eprintln!("ERROR: could not read the body of the request: {err}");
    })?;
    let body = str::from_utf8(&buf).map_err(|err| {
        eprintln!("ERROR: could not interpret body as UTF-8 string: {err}");
    })?.chars().collect::<Vec<_>>();

    let result = search_query(tf_index, &body);

    let json = serde_json::to_string(&result.iter().take(20).collect::<Vec<_>>()).map_err(|err| {
        eprintln!("ERROR: could not convert search results to JSON: {err}");
    })?;

    let content_type_header = Header::from_bytes("Content-Type", "application/json")
        .expect("That we didn't put any garbage in the headers");
    let response = Response::from_string(&json)
        .with_header(content_type_header);
    request.respond(response).map_err(|err| {
        eprintln!("ERROR: could not serve a request {err}");
    })
}

fn serve_request(tf_index: &TermFreqIndex, request: Request) -> Result<(), ()> {
    println!("INFO: received request! method: {:?}, url: {:?}", request.method(), request.url());

    match (request.method(), request.url()) {
        (Method::Post, "/api/search") => {
            serve_api_search(tf_index, request)
        }
        (Method::Get, "/index.js") => {
            serve_static_file(request, "index.js", "text/javascript; charset=utf-8")
        }
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            serve_static_file(request, "index.html", "text/html; charset=utf-8")
        }
        _ => {
            serve_404(request)
        }
    }
}

pub fn start(address: &str, tf_index: &TermFreqIndex) -> Result<(), ()> {
    let server = Server::http(&address).map_err(|err| {
        eprintln!("ERROR: could not start HTTP server at {address}: {err}");
    })?;

    println!("INFO: listening at http://{address}/");

    for request in server.incoming_requests() {
        // TODO: serve custom 500 in case of an error
        serve_request(&tf_index, request).ok();
    }

    eprintln!("ERROR: the server socket has shutdown");
    Err(())
}
