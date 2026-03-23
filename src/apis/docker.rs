use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use bollard::Docker;
use bollard::query_parameters::ListImagesOptions;
use serde::{Deserialize, Serialize};

use crate::utils::admin::verify_admin_access;
use crate::DbPool;

/// Docker 镜像信息结构体
#[derive(Serialize, Deserialize)]
pub struct DockerImage {
    pub id: String,
    pub parent_id: String,
    pub repo_tags: Vec<String>,
    pub repo_digests: Vec<String>,
    pub created: i64,
    pub size: i64,
    pub shared_size: i64,
    pub labels: Option<std::collections::HashMap<String, String>>,
    pub containers: i64,
}

/// 获取 Docker 镜像列表响应结构体
#[derive(Serialize)]
pub struct GetImagesResponse {
    pub success: bool,
    pub message: String,
    pub images: Vec<DockerImage>,
}

/// 获取 Docker 镜像列表（仅 admin 可访问）
///
/// # Endpoint
/// GET /docker/get_images
///
/// # Authorization
/// 需要 Bearer token，且用户类型必须为 admin
#[get("/docker/get_images")]
pub async fn get_images(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. 连接 Docker
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(e) => {
            return HttpResponse::InternalServerError().json(GetImagesResponse {
                success: false,
                message: format!("Failed to connect to Docker: {}", e),
                images: vec![],
            });
        }
    };

    // 3. 获取镜像列表
    let options = Some(ListImagesOptions {
        all: true,
        ..Default::default()
    });

    match docker.list_images(options).await {
        Ok(images) => {
            let docker_images: Vec<DockerImage> = images
                .into_iter()
                .map(|img| DockerImage {
                    id: img.id,
                    parent_id: img.parent_id,
                    repo_tags: img.repo_tags,
                    repo_digests: img.repo_digests,
                    created: img.created,
                    size: img.size,
                    shared_size: img.shared_size,
                    labels: Some(img.labels),
                    containers: img.containers,
                })
                .collect();

            HttpResponse::Ok().json(GetImagesResponse {
                success: true,
                message: "Images retrieved successfully".to_string(),
                images: docker_images,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(GetImagesResponse {
            success: false,
            message: format!("Failed to list images: {}", e),
            images: vec![],
        }),
    }
}
