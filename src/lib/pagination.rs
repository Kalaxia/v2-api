use actix_web::{http::StatusCode, HttpResponse};

#[derive(serde::Deserialize)]
pub struct Paginator{
    pub limit: i64,
    pub page: i64,
}

pub struct PaginatedResponse{}

impl PaginatedResponse {
    pub fn new<T: serde::Serialize>(limit: i64, page: i64, count: i64, content: T) -> HttpResponse {
        HttpResponse::build(StatusCode::PARTIAL_CONTENT)
            .header("pagination-count", count.to_string())
            .header("pagination-page", page.to_string())
            .header("pagination-limit", limit.to_string())
            .json(content)
    }
}