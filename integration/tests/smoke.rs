use rosso_integration::Stack;

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn status_reports_a_healthy_db() {
    let stack = Stack::start().await.unwrap();
    let body = stack.get_json("/status").await;
    assert_eq!(body["service"], "rosso");
    assert_eq!(body["db_healthy"], true);
    assert_eq!(body["llm_configured"], false);
    assert_eq!(body["llm_available"], false);
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn a_configured_but_unreachable_model_host_reports_unavailable() {
    // The distinction the header badge rests on: a URL in the environment is not
    // evidence that anything is listening at the other end.
    let dead = format!(
        "http://127.0.0.1:{}",
        rosso_integration::free_port().unwrap()
    );
    let stack = Stack::start_with_env(&[("ROSSO_OLLAMA_URL", &dead)])
        .await
        .unwrap();

    let body = stack.get_json("/status").await;
    assert_eq!(body["llm_configured"], true);
    assert_eq!(body["llm_available"], false);
    // And the reader is entirely unaffected by it.
    assert!(stack.get("/api/feeds").await.status().is_success());
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn unknown_route_serves_the_spa_shell_with_200() {
    // The regression this guards: serving the shell through
    // ServeDir.not_found_service returns it with a 404, so a hard refresh on a
    // client route looks broken to anything that checks the status.
    let stack = Stack::start().await.unwrap();
    let res = stack.get("/feeds/42").await;
    assert!(res.status().is_success(), "status was {}", res.status());
    assert!(res.text().await.unwrap().contains("rosso"));
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn csp_covers_the_spa_document_and_not_just_the_api() {
    // `Router::layer` wraps only what is already in the router, so a layer added
    // before `.fallback` silently skips the SPA document — the one response the
    // CSP exists to protect.
    let stack = Stack::start().await.unwrap();
    for route in ["/status", "/", "/feeds/42"] {
        let res = stack.get(route).await;
        assert!(
            res.headers().contains_key("content-security-policy"),
            "no CSP on {route}"
        );
    }
}

#[tokio::test]
#[ignore = "spawns the backend binary"]
async fn path_traversal_out_of_static_dir_is_refused() {
    let stack = Stack::start().await.unwrap();
    let body = stack.get("/../../etc/passwd").await.text().await.unwrap();
    assert!(!body.contains("root:"), "served a file outside static_dir");
}
