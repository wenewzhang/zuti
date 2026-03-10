use actix_web::{get, App, HttpResponse, HttpServer, Responder};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde::Serialize;

#[derive(Serialize)]
struct PingResponse {
    status: String,
    message: String,
}

#[get("/ping")]
async fn ping() -> impl Responder {
    HttpResponse::Ok().json(PingResponse {
        status: "ok".to_string(),
        message: "pong".to_string(),
    })
}

#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok().body("HTTPS Server is running!")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 加载 TLS 证书
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("certs/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("certs/cert.pem")
        .unwrap();

    println!("HTTPS Server running at https://127.0.0.1:8443");
    println!("Try: curl -k https://localhost:8443/ping");

    HttpServer::new(|| {
        App::new()
            .service(index)
            .service(ping)
    })
    .bind_openssl("127.0.0.1:8443", builder)?
    .run()
    .await
}
