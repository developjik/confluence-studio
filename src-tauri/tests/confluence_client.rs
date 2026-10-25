//! Integration tests for the Confluence client against a wiremock server.
//! Covers the contract cases in docs/ARCHITECTURE.md § "Module: confluence":
//! space listing (cursor, 2 pages), page tree (3 levels), get (markdown
//! conversion), create/update/delete request bodies (storage representation,
//! version+1) and the 401 error message.

use confluence_studio_lib::confluence::convert;
use confluence_studio_lib::confluence::{ConfluenceAuth, ConfluenceClient, ConfluenceError};
use serde_json::{Value, json};
use wiremock::matchers::{
    basic_auth, body_json, method, path, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockGuard, MockServer, ResponseTemplate};

fn client_for(server: &MockServer, base_suffix: &str) -> ConfluenceClient {
    ConfluenceClient::new(ConfluenceAuth {
        base_url: format!("{}{}", server.uri(), base_suffix),
        email: "dev@example.com".to_string(),
        token: "secret-token".to_string(),
    })
}

async fn get_json_mock(server: &MockServer, route: &str, body: Value) -> MockGuard {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount_as_scoped(server)
        .await
}

#[tokio::test]
async fn list_spaces_paginates_with_cursor() {
    let server = MockServer::start().await;
    // base_url carrying a "/wiki" suffix must be normalized away
    let client = client_for(&server, "/wiki");

    let page1 = Mock::given(method("GET"))
        .and(path("/wiki/api/v2/spaces"))
        .and(query_param("limit", "50"))
        .and(query_param_is_missing("cursor"))
        .and(basic_auth("dev@example.com", "secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"id": "10", "key": "ENG", "name": "Engineering", "type": "global"}
            ],
            "_links": {"next": "/wiki/api/v2/spaces?cursor=abc123"}
        })))
        .expect(1)
        .mount_as_scoped(&server)
        .await;
    let page2 = Mock::given(method("GET"))
        .and(path("/wiki/api/v2/spaces"))
        .and(query_param("cursor", "abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"id": "20", "key": "OPS", "name": "Operations", "type": "global"}
            ],
            "_links": {}
        })))
        .expect(1)
        .mount_as_scoped(&server)
        .await;

    let spaces = client
        .list_spaces()
        .await
        .expect("list_spaces should succeed");
    assert_eq!(spaces.len(), 2);
    assert_eq!(spaces[0].id, "10");
    assert_eq!(spaces[0].key.as_deref(), Some("ENG"));
    assert_eq!(spaces[0].name, "Engineering");
    assert_eq!(spaces[1].name, "Operations");

    page1.wait_until_satisfied().await;
    page2.wait_until_satisfied().await;
}

#[tokio::test]
async fn page_tree_builds_three_levels() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");
    let page = |id: &str, title: &str| json!({"id": id, "title": title, "status": "current"});

    let g1 = get_json_mock(
        &server,
        "/wiki/api/v2/spaces/SP1/pages",
        json!({"results": [page("A", "Root page")], "_links": {}}),
    )
    .await;
    let g2 = get_json_mock(
        &server,
        "/wiki/api/v2/pages/A/children",
        json!({"results": [page("B", "Level 2")], "_links": {}}),
    )
    .await;
    let g3 = get_json_mock(
        &server,
        "/wiki/api/v2/pages/B/children",
        json!({"results": [page("C", "Level 3")], "_links": {}}),
    )
    .await;
    let g4 = get_json_mock(
        &server,
        "/wiki/api/v2/pages/C/children",
        json!({"results": [], "_links": {}}),
    )
    .await;

    let tree = client
        .page_tree("SP1")
        .await
        .expect("page_tree should succeed");
    assert_eq!(tree.id, "SP1");
    assert_eq!(tree.status, "space");
    assert_eq!(tree.children.len(), 1);

    let a = &tree.children[0];
    assert_eq!((a.id.as_str(), a.title.as_str()), ("A", "Root page"));
    assert_eq!(a.children.len(), 1);

    let b = &a.children[0];
    assert_eq!((b.id.as_str(), b.title.as_str()), ("B", "Level 2"));

    let c = &b.children[0];
    assert_eq!((c.id.as_str(), c.title.as_str()), ("C", "Level 3"));
    assert!(c.children.is_empty(), "level 3 must be the deepest level");

    g1.wait_until_satisfied().await;
    g2.wait_until_satisfied().await;
    g3.wait_until_satisfied().await;
    g4.wait_until_satisfied().await;
}

#[tokio::test]
async fn get_page_converts_storage_to_markdown() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let g = Mock::given(method("GET"))
        .and(path("/wiki/api/v2/pages/P1"))
        .and(query_param("body-format", "storage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "P1",
            "status": "current",
            "title": "Sample",
            "spaceId": "SP9",
            "parentId": "P0",
            "createdAt": "2024-05-01T10:00:00Z",
            "version": {"number": 7, "createdAt": "2024-06-01T10:00:00Z"},
            "body": {"storage": {
                "value": "<h1>Sample</h1><p>Hello <strong>world</strong></p>",
                "representation": "storage"
            }},
            "_links": {"webui": "/spaces/KEY/pages/P1"}
        })))
        .mount_as_scoped(&server)
        .await;

    let d = client
        .get_page("P1")
        .await
        .expect("get_page should succeed");
    assert_eq!(d.id, "P1");
    assert_eq!(d.title, "Sample");
    assert_eq!(d.space_id, "SP9");
    assert_eq!(d.parent_id.as_deref(), Some("P0"));
    assert_eq!(d.version, 7);
    assert_eq!(d.markdown, "# Sample\n\nHello **world**\n");
    assert_eq!(
        d.web_url,
        format!("{}/wiki/spaces/KEY/pages/P1", server.uri())
    );
    assert_eq!(d.created_at.as_deref(), Some("2024-05-01T10:00:00Z"));
    assert_eq!(d.updated_at.as_deref(), Some("2024-06-01T10:00:00Z"));

    g.wait_until_satisfied().await;
}

#[tokio::test]
async fn create_page_sends_storage_representation() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let expected = json!({
        "spaceId": "SP1",
        "status": "current",
        "title": "New page",
        "body": {
            "representation": "storage",
            "value": convert::markdown_to_storage("Markdown **body**")
        }
    });
    let g = Mock::given(method("POST"))
        .and(path("/wiki/api/v2/pages"))
        .and(body_json(expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "NEW1",
            "status": "current",
            "title": "New page",
            "spaceId": "SP1",
            "version": {"number": 1},
            "body": {"storage": {
                "value": "<p>Markdown <strong>body</strong></p>",
                "representation": "storage"
            }},
            "_links": {"webui": "/spaces/KEY/pages/NEW1"}
        })))
        .mount_as_scoped(&server)
        .await;

    let d = client
        .create_page("SP1", None, "New page", "Markdown **body**")
        .await
        .expect("create_page should succeed");
    assert_eq!(d.id, "NEW1");
    assert_eq!(d.version, 1);
    assert_eq!(d.markdown, "Markdown **body**\n");

    g.wait_until_satisfied().await;
}

#[tokio::test]
async fn create_page_with_parent_sends_parent_id() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let expected = json!({
        "spaceId": "SP1",
        "status": "current",
        "parentId": "PARENT",
        "title": "Child page",
        "body": {
            "representation": "storage",
            "value": convert::markdown_to_storage("child")
        }
    });
    let g = Mock::given(method("POST"))
        .and(path("/wiki/api/v2/pages"))
        .and(body_json(expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "NEW2",
            "status": "current",
            "title": "Child page",
            "spaceId": "SP1",
            "parentId": "PARENT",
            "version": {"number": 1}
        })))
        .mount_as_scoped(&server)
        .await;

    let d = client
        .create_page("SP1", Some("PARENT"), "Child page", "child")
        .await
        .expect("create_page should succeed");
    assert_eq!(d.id, "NEW2");
    assert_eq!(d.parent_id.as_deref(), Some("PARENT"));

    g.wait_until_satisfied().await;
}

#[tokio::test]
async fn update_page_increments_version() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let expected = json!({
        "id": "P2",
        "status": "current",
        "title": "Updated",
        "body": {
            "representation": "storage",
            "value": convert::markdown_to_storage("Updated content")
        },
        "version": {"number": 8}
    });
    let g = Mock::given(method("PUT"))
        .and(path("/wiki/api/v2/pages/P2"))
        .and(body_json(expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "P2",
            "status": "current",
            "title": "Updated",
            "spaceId": "SP1",
            "version": {"number": 8},
            "body": {"storage": {
                "value": "<p>Updated content</p>",
                "representation": "storage"
            }}
        })))
        .mount_as_scoped(&server)
        .await;

    let d = client
        .update_page("P2", "Updated", "Updated content", 7)
        .await
        .expect("update_page should succeed");
    // the returned detail must carry the NEW version
    assert_eq!(d.version, 8);
    assert_eq!(d.markdown, "Updated content\n");

    g.wait_until_satisfied().await;
}

#[tokio::test]
async fn delete_page_succeeds() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let g = Mock::given(method("DELETE"))
        .and(path("/wiki/api/v2/pages/P3"))
        .respond_with(ResponseTemplate::new(204))
        .mount_as_scoped(&server)
        .await;

    client
        .delete_page("P3")
        .await
        .expect("delete_page should succeed");

    g.wait_until_satisfied().await;
}

#[tokio::test]
async fn unauthorized_maps_to_auth_error_message() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let _g = Mock::given(method("GET"))
        .and(path("/wiki/api/v2/spaces"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"message": "nope"})))
        .mount_as_scoped(&server)
        .await;

    let err = client
        .list_spaces()
        .await
        .expect_err("401 must be an error");
    let msg = err.to_string();
    assert!(matches!(err, ConfluenceError::Auth { status: 401, .. }));
    assert!(msg.starts_with("인증 실패(401)"), "got: {msg}");
}

#[tokio::test]
async fn missing_page_maps_to_not_found_error_message() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let _g = Mock::given(method("GET"))
        .and(path("/wiki/api/v2/pages/NOPE"))
        .respond_with(ResponseTemplate::new(404).set_body_string("{\"message\":\"none\"}"))
        .mount_as_scoped(&server)
        .await;

    let err = client
        .get_page("NOPE")
        .await
        .expect_err("404 must be an error");
    let msg = err.to_string();
    assert!(matches!(err, ConfluenceError::NotFound));
    assert_eq!(msg, "페이지/스페이스 없음");
}

#[tokio::test]
async fn test_connection_returns_ok() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let g = Mock::given(method("GET"))
        .and(path("/wiki/api/v2/spaces"))
        .and(query_param("limit", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results": [], "_links": {}}))
                .insert_header("date", "Mon, 01 Jan 2024 00:00:00 GMT"),
        )
        .mount_as_scoped(&server)
        .await;

    let r = client.test().await.expect("test should succeed");
    assert!(r.ok);
    assert_eq!(
        r.server_time.as_deref(),
        Some("Mon, 01 Jan 2024 00:00:00 GMT")
    );

    g.wait_until_satisfied().await;
}

#[tokio::test]
async fn list_attachments_and_download() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let g = get_json_mock(
        &server,
        "/wiki/rest/api/content/C1/child/attachment",
        json!({
            "results": [{
                "id": "att100",
                "title": "shot.png",
                "extensions": {"mediaType": "image/png"},
                "_links": {"download": "/wiki/download/attachments/C1/shot.png"}
            }],
            "_links": {}
        }),
    )
    .await;
    let gd = Mock::given(method("GET"))
        .and(path("/wiki/download/attachments/C1/shot.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8, 2, 3, 4]))
        .mount_as_scoped(&server)
        .await;

    let atts = client
        .list_attachments("C1")
        .await
        .expect("list_attachments should succeed");
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].id, "att100");
    assert_eq!(atts[0].title, "shot.png");
    assert_eq!(atts[0].media_type, "image/png");
    assert_eq!(
        atts[0].download_url,
        "/wiki/download/attachments/C1/shot.png"
    );

    let bytes = client
        .download_attachment("/wiki/download/attachments/C1/shot.png")
        .await
        .expect("download should succeed");
    assert_eq!(bytes, vec![1u8, 2, 3, 4]);

    g.wait_until_satisfied().await;
    gd.wait_until_satisfied().await;
}

#[tokio::test]
async fn get_descendants_returns_briefs() {
    let server = MockServer::start().await;
    let client = client_for(&server, "");

    let g = get_json_mock(
        &server,
        "/wiki/api/v2/pages/A/descendants",
        json!({
            "results": [
                {"id": "B", "title": "Child", "status": "current"},
                {"id": "C", "title": "Grandchild", "status": "current"}
            ],
            "_links": {}
        }),
    )
    .await;

    let d = client
        .get_descendants("A")
        .await
        .expect("get_descendants should succeed");
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].id, "B");
    assert_eq!(d[1].title, "Grandchild");

    g.wait_until_satisfied().await;
}
