use actix_web::{get, App, HttpResponse, HttpServer, Responder};

#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

#[get("/again")]
async fn index2() -> impl Responder {
    HttpResponse::Ok().body("Hello world again!")
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(index)
            .service(index2)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}