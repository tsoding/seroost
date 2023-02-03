use std::fs::File;
use std::str;
use std::io;

use super::model::*;

use tiny_http::{Server, Request, Response, Header, Method, StatusCode};

fn serve_404(request: Request) -> io::Result<()> {
    request.respond(Response::from_string("404").with_status_code(StatusCode(404)))
}

fn serve_500(request: Request) -> io::Result<()> {
    request.respond(Response::from_string("500").with_status_code(StatusCode(500)))
}

fn serve_400(request: Request, message: &str) -> io::Result<()> {
    request.respond(Response::from_string(format!("400: {message}")).with_status_code(StatusCode(400)))
}

fn serve_static_file(request: Request, file_path: &str, content_type: &str) -> io::Result<()> {
    let content_type_header = Header::from_bytes("Content-Type", content_type)
        .expect("That we didn't put any garbage in the headers");

    let file = match File::open(file_path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("ERROR: could not serve file {file_path}: {err}");
            if err.kind() == io::ErrorKind::NotFound {
                return serve_404(request);
            }
            return serve_500(request);
        }
    };

    request.respond(Response::from_file(file).with_header(content_type_header))
}

// TODO: the errors of serve_api_search should probably return JSON
// 'Cause that's what expected from them.
fn serve_api_search(model: &impl Model, mut request: Request) -> io::Result<()> {
    let mut buf = Vec::new();
    if let Err(err) = request.as_reader().read_to_end(&mut buf) {
        eprintln!("ERROR: could not read the body of the request: {err}");
        return serve_500(request);
    }

    let body = match str::from_utf8(&buf) {
        Ok(body) => body.chars().collect::<Vec<_>>(),
        Err(err) => {
            eprintln!("ERROR: could not interpret body as UTF-8 string: {err}");
            return serve_400(request, "Body must be a valid UTF-8 string");
        }
    };

    let result = match model.search_query(&body) {
        Ok(result) => result,
        Err(()) => return serve_500(request),
    };

    let json = match serde_json::to_string(&result.iter().take(20).collect::<Vec<_>>()) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("ERROR: could not convert search results to JSON: {err}");
            return serve_500(request)
        }
    };

    let content_type_header = Header::from_bytes("Content-Type", "application/json")
        .expect("That we didn't put any garbage in the headers");
    request.respond(Response::from_string(&json).with_header(content_type_header))
}

fn serve_request(model: &impl Model, request: Request) -> io::Result<()> {
    println!("INFO: received request! method: {:?}, url: {:?}", request.method(), request.url());

    match (request.method(), request.url()) {
        (Method::Post, "/api/search") => {
            serve_api_search(model, request)
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

pub fn start(address: &str, model: &impl Model) -> Result<(), ()> {
    let server = Server::http(&address).map_err(|err| {
        eprintln!("ERROR: could not start HTTP server at {address}: {err}");
    })?;

    println!("INFO: listening at http://{address}/");

    for request in server.incoming_requests() {
        serve_request(model, request).map_err(|err| {
            eprintln!("ERROR: could not serve the response: {err}");
        }).ok(); // <- don't stop on errors, keep serving
    }

    eprintln!("ERROR: the server socket has shutdown");
    Err(())
}
