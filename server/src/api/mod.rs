pub mod providers;
pub mod sync;
pub mod webhook;

use actix_web::web;

/// Registers all API routes under `/api`.
pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/providers")
            .route("", web::get().to(providers::list))
            .route("/{domain}", web::get().to(providers::detail))
            .route("/{domain}/history", web::get().to(providers::history)),
    )
    .service(
        web::scope("/sync")
            .route("/status", web::get().to(sync::status))
            .route("/{domain}", web::post().to(sync::trigger)),
    )
    .service(web::scope("/webhook").route("/github", web::post().to(webhook::github)));
}
