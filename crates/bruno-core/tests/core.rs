use bruno_core::collection::{load_collection, ROOT_FILE};
use bruno_core::interpolate::{interpolate, merge_vars, run_assertions, Vars};
use bruno_core::model::Assertion;
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")
}

#[test]
fn parses_opencollection_root() {
    let dir = fixture_dir();
    assert!(dir.join(ROOT_FILE).is_file());
    let collection = load_collection(&dir).expect("load collection");
    assert_eq!(collection.name, "Bruno RS Demo");
    assert_eq!(collection.root.version, "1.0.0");
    assert_eq!(collection.tree.len(), 1, "应扫描到 1 个请求");
    assert_eq!(collection.tree[0].kind, "http-request");
    assert_eq!(collection.tree[0].name, "Echo GET");
    assert!(collection.requests.contains_key("echo.yml"));
}

#[test]
fn interpolates_variables_with_priority() {
    let mut low = Vars::new();
    low.insert("host".into(), "http://low".into());
    let mut high = Vars::new();
    high.insert("host".into(), "http://high".into());
    let merged = merge_vars(&[low, high]);
    assert_eq!(
        interpolate("{{host}}/users/{{missing}}", &merged),
        "http://high/users/{{missing}}"
    );
}

#[test]
fn runs_declarative_assertions() {
    let assertions = vec![
        Assertion {
            expression: "res.status".into(),
            operator: "eq".into(),
            value: Some("200".into()),
            disabled: None,
            description: None,
        },
        Assertion {
            expression: "res.body.ok".into(),
            operator: "eq".into(),
            value: Some("true".into()),
            disabled: None,
            description: None,
        },
    ];
    let body = serde_json::json!({ "ok": true });
    let results = run_assertions(&assertions, 200, "OK", r#"{"ok":true}"#, Some(&body));
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.passed), "全部断言应通过：{results:?}");

    let failed = run_assertions(&assertions, 500, "Error", r#"{"ok":false}"#, None);
    assert!(failed.iter().all(|r| !r.passed), "状态码与 body 都不匹配");
}

#[test]
fn tree_nodes_omit_keys_the_frontend_uses_for_discrimination() {
    // Bruno 前端判定规则：
    //   isItemARequest = hasOwnProperty('request') && !items
    //   isItemAFolder  = !hasOwnProperty('request') && type === 'folder'
    // 所以请求节点不能带空 items，文件夹节点不能带 request 键（null 也不行）。
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/shop");
    let collection = load_collection(&dir).expect("load shop collection");

    let request = collection
        .tree
        .iter()
        .find(|n| n.kind == "http-request")
        .expect("应有请求节点");
    let request_json = serde_json::to_value(request).unwrap();
    assert!(request_json.get("items").is_none(), "请求节点不能有 items 键：{request_json}");
    assert!(request_json.get("request").is_some(), "请求节点必须有 request 键");

    let folder = collection
        .tree
        .iter()
        .find(|n| n.kind == "folder")
        .expect("应有文件夹节点");
    let folder_json = serde_json::to_value(folder).unwrap();
    assert!(folder_json.get("request").is_none(), "文件夹不能有 request 键：{folder_json}");
    assert!(folder_json.get("items").is_some(), "文件夹必须带 items 键");
    assert_eq!(folder.items.len(), 1, "文件夹里应有 1 个请求");
}

#[test]
fn request_file_persists_yaml_shape() {
    let dir = fixture_dir();
    let raw = fs::read_to_string(dir.join("echo.yml")).unwrap();
    let req: bruno_core::model::HttpRequest = serde_yml::from_str(&raw).unwrap();
    assert_eq!(req.http.as_ref().unwrap().method.as_deref(), Some("GET"));
    assert_eq!(
        req.http.as_ref().unwrap().url.as_deref(),
        Some("http://127.0.0.1:18081/hello")
    );
    assert_eq!(req.runtime.assertions.len(), 2);
}

#[test]
fn saves_then_reloads_from_disk() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scripted");
    let dst = std::env::temp_dir().join("bruno-rs-save-reload");
    let _ = fs::remove_dir_all(&dst);
    fs::create_dir_all(&dst).unwrap();
    for entry in fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
    }

    let collection = load_collection(&dst).expect("load scripted");
    let file = collection.requests.get("scripted.yml").expect("scripted.yml");
    let ui = bruno_core::collection::normalize_request_for_ui(&file.request);
    let item = serde_json::json!({
        "uid": file.uid,
        "type": "http-request",
        "name": file.name,
        "seq": file.seq,
        "request": {
            "method": ui["method"],
            "url": "http://127.0.0.1:18081/changed",
            "headers": ui["headers"],
            "params": ui["params"],
            "body": ui["body"],
            "auth": ui["auth"],
            "vars": ui["vars"],
            "script": ui["script"],
            "tests": ui["tests"],
            "assertions": ui["assertions"],
        },
        "settings": ui_settings(&file.request),
    });
    bruno_core::save_request_file(&dst.join("scripted.yml"), &item).expect("save");

    // 重新从磁盘加载（等价重启应用）
    let reloaded = load_collection(&dst).expect("reload");
    let req = &reloaded.requests["scripted.yml"].request;
    assert_eq!(
        req.http.as_ref().unwrap().url.as_deref(),
        Some("http://127.0.0.1:18081/changed")
    );
    assert_eq!(req.runtime.scripts.len(), 2, "before-request + tests 都要保留");
    assert_eq!(req.runtime.assertions.len(), 1);
    let raw = fs::read_to_string(dst.join("scripted.yml")).unwrap();
    assert!(!raw.contains("null"), "YAML 不应出现 null：\n{raw}");
    assert!(!raw.contains("encode_url"), "不应写回 snake_case 别名：\n{raw}");

    // multipart 请求体的磁盘 ↔ UI 形状
    let upload = reloaded.requests.get("upload.yml").expect("upload.yml");
    let ui = bruno_core::collection::normalize_request_for_ui(&upload.request);
    assert_eq!(ui["body"]["mode"], "multipartForm");
    assert_eq!(ui["body"]["multipartForm"][0]["name"], "title");
    assert_eq!(ui["body"]["multipartForm"][1]["value"], "{{scriptTag}}");
}

fn ui_settings(req: &bruno_core::model::HttpRequest) -> serde_json::Value {
    bruno_core::collection::settings_to_ui(&req.settings)
}
