use actix_web::{http::StatusCode, HttpResponse};

#[derive(serde::Deserialize)]
pub struct Paginator{
    pub limit: i64,
    pub page: i64,
}

pub struct PaginatedResponse{}

impl PaginatedResponse {
    pub fn new<T: serde::Serialize>(limit: i64, page: i64, count: i64, content: Vec<T>) -> HttpResponse {
        let first = page - 1 * limit;
        let last = first + content.len() as i64;
        HttpResponse::build(StatusCode::PARTIAL_CONTENT)
            .header("content-range", format!("{}-{}/{}", first, last, count))
            .json(content)
    }
}