use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::utils::admin::validate_token_with_db;
use crate::DbPool;

/// Log query parameters
#[derive(Deserialize, Debug)]
pub struct LogQuery {
    /// Page number (1-based)
    #[serde(default = "default_log_page")]
    pub page: usize,
    /// Lines per page
    #[serde(default = "default_log_page_size")]
    pub page_size: usize,
}

fn default_log_page() -> usize {
    1
}

fn default_log_page_size() -> usize {
    100
}

/// Log response
#[derive(Serialize)]
pub struct LogResponse {
    pub success: bool,
    pub message: String,
    pub log_file: String,
    pub logs: Vec<String>,
    pub total_lines: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
}

fn read_log_file(path: &str, query: &LogQuery) -> LogResponse {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return LogResponse {
                success: false,
                message: format!("Failed to read log file: {}", e),
                log_file: path.to_string(),
                logs: vec![],
                total_lines: 0,
                page: query.page,
                page_size: query.page_size,
                total_pages: 0,
            };
        }
    };

    let all_lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let total_lines = all_lines.len();

    let page_size = query.page_size.max(1);
    let page = query.page.max(1);
    let total_pages = if total_lines == 0 {
        1
    } else {
        ((total_lines + page_size - 1) / page_size).max(1)
    };

    let start = (page - 1) * page_size;
    let logs = if start >= total_lines {
        vec![]
    } else {
        let end = (start + page_size).min(total_lines);
        all_lines[start..end].to_vec()
    };

    LogResponse {
        success: true,
        message: format!(
            "Retrieved {} lines, showing page {} of {}",
            total_lines, page, total_pages
        ),
        log_file: path.to_string(),
        logs,
        total_lines,
        page,
        page_size,
        total_pages,
    }
}

/// GET /log/zuti
#[get("/log/zuti")]
pub async fn get_zuti_log(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    query: web::Query<LogQuery>,
) -> impl Responder {
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }
    let response = read_log_file("/var/log/zuti/zuti.log", &query);
    HttpResponse::Ok().json(response)
}

/// GET /log/zuti-helper
#[get("/log/zuti-helper")]
pub async fn get_zuti_helper_log(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    query: web::Query<LogQuery>,
) -> impl Responder {
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }
    let response = read_log_file("/var/log/zuti/zuti-helper.log", &query);
    HttpResponse::Ok().json(response)
}

/// GET /log/zuti-updater
#[get("/log/zuti-updater")]
pub async fn get_zuti_updater_log(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    query: web::Query<LogQuery>,
) -> impl Responder {
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }
    let response = read_log_file("/var/log/zuti/zuti-updater.log", &query);
    HttpResponse::Ok().json(response)
}
