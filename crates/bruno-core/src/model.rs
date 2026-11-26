//! OpenCollection YAML 数据模型。
//! 严格对齐 @opencollection/types@0.15.0，但刻意宽容：
//! - 字段全部 `#[serde(default)]`（真实 fixture 中有 schema 未声明的键）
//! - 不用 `deny_unknown_fields`（fixture 里有 `enabled` 等多余键）
//! - 版本字段用 String（yml 里 `1.0.0` 与 `"1.0.0"` 都会解析为字符串）
//! - 多态字段用 `#[serde(untagged)]`

use serde::{Deserialize, Serialize};

// ============================================================================
// 通用多态类型
// ============================================================================

/// 描述：纯字符串 或 { content, type }
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Description {
    Structured {
        content: String,
        #[serde(default)]
        r#type: Option<String>,
    },
    Text(String),
    #[default]
    None,
}

impl Description {
    pub fn text(&self) -> String {
        match self {
            Description::Text(s) => s.clone(),
            Description::Structured { content, .. } => content.clone(),
            Description::None => String::new(),
        }
    }
}

/// 类型化值：{ type, data }，data 始终是 YAML 字符串
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedValue {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub data: String,
}

/// 变量值：string | {type,data} | [{title,selected,value}]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VariableValue {
    Typed(TypedValue),
    Variants(Vec<ValueVariant>),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueVariant {
    pub title: String,
    #[serde(default)]
    pub selected: Option<bool>,
    #[serde(default)]
    pub value: Option<VariableValue>,
}

/// 请求级变量 / 环境变量成员
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    #[serde(default)]
    pub value: Option<VariableValue>,
    #[serde(default)]
    pub description: Option<Description>,
    #[serde(default)]
    pub disabled: Option<bool>,
    /// 环境变量专用：secret 变量不带 value
    #[serde(default)]
    pub secret: Option<bool>,
}

impl Variable {
    /// 解析为插值用的字符串值（类型化值按 type 归一化为 data）
    pub fn resolved(&self) -> Option<String> {
        if self.secret == Some(true) {
            return None;
        }
        match &self.value {
            Some(VariableValue::Text(s)) => Some(s.clone()),
            Some(VariableValue::Typed(t)) => Some(t.data.clone()),
            Some(VariableValue::Variants(variants)) => variants
                .iter()
                .find(|v| v.selected == Some(true))
                .or_else(|| variants.first())
                .and_then(|v| v.value.as_ref())
                .and_then(|value| match value {
                    VariableValue::Text(s) => Some(s.clone()),
                    VariableValue::Typed(t) => Some(t.data.clone()),
                    VariableValue::Variants(_) => None,
                }),
            None => None,
        }
    }
}

// ============================================================================
// 认证（untagged，靠 type 区分；`auth: inherit` 是纯字符串）
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Auth {
    Inherit(String),
    Typed(AuthTyped),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthTyped {
    #[serde(rename = "basic")]
    Basic {
        #[serde(default)] username: Option<String>,
        #[serde(default)] password: Option<String>,
    },
    #[serde(rename = "bearer")]
    Bearer {
        #[serde(default)] token: Option<String>,
    },
    #[serde(rename = "apikey")]
    ApiKey {
        #[serde(default)] key: Option<String>,
        #[serde(default)] value: Option<String>,
        /// header | query（真实 fixture 里是 query）
        #[serde(default)] placement: Option<String>,
    },
    #[serde(rename = "digest")]
    Digest {
        #[serde(default)] username: Option<String>,
        #[serde(default)] password: Option<String>,
    },
    #[serde(rename = "ntlm")]
    Ntlm {
        #[serde(default)] username: Option<String>,
        #[serde(default)] password: Option<String>,
        #[serde(default)] domain: Option<String>,
    },
    #[serde(rename = "wsse")]
    Wsse {
        #[serde(default)] username: Option<String>,
        #[serde(default)] password: Option<String>,
    },
    #[serde(rename = "awsv4")]
    AwsV4 {
        #[serde(default, rename = "accessKeyId")] access_key_id: Option<String>,
        #[serde(default, rename = "secretAccessKey")] secret_access_key: Option<String>,
        #[serde(default, rename = "sessionToken")] session_token: Option<String>,
        #[serde(default)] service: Option<String>,
        #[serde(default)] region: Option<String>,
        #[serde(default, rename = "profileName")] profile_name: Option<String>,
    },
    #[serde(rename = "oauth2")]
    OAuth2(serde_json::Value),
    #[serde(rename = "oauth1")]
    OAuth1(serde_json::Value),
    /// Bruno 专有（非 OpenCollection 标准），先原样保留
    #[serde(rename = "akamai-edgegrid")]
    EdgeGrid(serde_json::Value),
    /// 未知认证类型：宽容接收，避免整份集合解析失败
    #[serde(other)]
    Unknown,
}

impl AuthTyped {
    pub fn type_name(&self) -> &'static str {
        match self {
            AuthTyped::Basic { .. } => "basic",
            AuthTyped::Bearer { .. } => "bearer",
            AuthTyped::ApiKey { .. } => "apikey",
            AuthTyped::Digest { .. } => "digest",
            AuthTyped::Ntlm { .. } => "ntlm",
            AuthTyped::Wsse { .. } => "wsse",
            AuthTyped::AwsV4 { .. } => "awsv4",
            AuthTyped::OAuth2(_) => "oauth2",
            AuthTyped::OAuth1(_) => "oauth1",
            AuthTyped::EdgeGrid(_) => "akamai-edgegrid",
            AuthTyped::Unknown => "unknown",
        }
    }
}

// ============================================================================
// HTTP 请求
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpHeader {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub description: Option<Description>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpParam {
    pub name: String,
    #[serde(default)]
    pub value: String,
    /// query | path
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BodyData {
    Raw { r#type: String, data: String },
    Multipart { r#type: String, data: Vec<MultipartEntry> },
    FormUrl { r#type: String, data: Vec<FormEntry> },
    File { r#type: String, data: serde_json::Value },
    /// 变体数组 [{title, selected, body}]
    Variants(Vec<serde_json::Value>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipartEntry {
    pub name: String,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormEntry {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

impl BodyData {
    pub fn type_name(&self) -> &str {
        match self {
            BodyData::Raw { r#type, .. }
            | BodyData::Multipart { r#type, .. }
            | BodyData::FormUrl { r#type, .. }
            | BodyData::File { r#type, .. } => r#type,
            BodyData::Variants(_) => "variants",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HttpDetails {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    #[serde(default)]
    pub params: Vec<HttpParam>,
    #[serde(default)]
    pub body: Option<BodyData>,
    #[serde(default)]
    pub auth: Option<Auth>,
}

/// bool | "inherit"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BoolOrInherit {
    Bool(bool),
    Inherit(String),
}

/// number | "inherit"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NumOrInherit {
    Num(i64),
    Inherit(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HttpSettings {
    #[serde(default)]
    pub encode_url: Option<BoolOrInherit>,
    #[serde(default, rename = "encodeUrl")]
    pub encode_url_camel: Option<BoolOrInherit>,
    #[serde(default)]
    pub timeout: Option<NumOrInherit>,
    #[serde(default)]
    pub follow_redirects: Option<BoolOrInherit>,
    #[serde(default, rename = "followRedirects")]
    pub follow_redirects_camel: Option<BoolOrInherit>,
    #[serde(default)]
    pub max_redirects: Option<NumOrInherit>,
    #[serde(default, rename = "maxRedirects")]
    pub max_redirects_camel: Option<NumOrInherit>,
    #[serde(default)]
    pub omit_headers: Vec<String>,
    #[serde(default, rename = "omitHeaders")]
    pub omit_headers_camel: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assertion {
    pub expression: String,
    pub operator: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(default)]
    pub description: Option<Description>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    /// before-request | after-response | tests | hooks | ...
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Runtime {
    #[serde(default)]
    pub variables: Vec<Variable>,
    #[serde(default)]
    pub scripts: Vec<Script>,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(default)]
    pub actions: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Info {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub seq: Option<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: Option<Description>,
}

/// WebSocket / gRPC 的消息体：单条 `{type,data}` 或变体数组
/// `[{title, selected, message:{type,data}}]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProtocolMessage {
    /// gRPC 单条消息在磁盘上直接是字符串（`message: '{"x":1}'`）
    Text(String),
    Typed {
        #[serde(default, rename = "type")]
        kind: Option<String>,
        #[serde(default)]
        data: String,
    },
    Variants(Vec<serde_json::Value>),
}

/// `websocket:` 段（OpenCollection 的 WebSocket 请求）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WebSocketDetails {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    #[serde(default)]
    pub message: Option<ProtocolMessage>,
    #[serde(default)]
    pub auth: Option<Auth>,
}

/// `grpc:` 段
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GrpcDetails {
    #[serde(default)]
    pub url: Option<String>,
    /// `/pkg.Service/Method`
    #[serde(default)]
    pub method: Option<String>,
    /// unary | client-streaming | server-streaming | bidi-streaming
    #[serde(default, rename = "methodType")]
    pub method_type: Option<String>,
    /// 相对集合目录的 .proto 路径；空表示走 server reflection
    #[serde(default, rename = "protoFilePath")]
    pub proto_file_path: Option<String>,
    #[serde(default)]
    pub metadata: Vec<HttpHeader>,
    #[serde(default)]
    pub message: Option<ProtocolMessage>,
    #[serde(default)]
    pub auth: Option<Auth>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HttpRequest {
    #[serde(default)]
    pub info: Info,
    #[serde(default)]
    pub http: Option<HttpDetails>,
    /// WebSocket 请求（info.type == "websocket"）
    #[serde(default)]
    pub websocket: Option<WebSocketDetails>,
    /// gRPC 请求（info.type == "grpc"）
    #[serde(default)]
    pub grpc: Option<GrpcDetails>,
    #[serde(default)]
    pub runtime: Runtime,
    #[serde(default)]
    pub settings: HttpSettings,
    #[serde(default)]
    pub docs: Option<String>,
}

impl HttpRequest {
    /// info.type → 前端 kind（`http-request` / `ws-request` / `grpc-request`）
    pub fn kind(&self) -> &'static str {
        match self.info.r#type.as_deref().unwrap_or("http") {
            "websocket" | "ws" => "ws-request",
            "grpc" => "grpc-request",
            _ => "http-request",
        }
    }
}

// ============================================================================
// 集合 / 文件夹 / 环境
// ============================================================================

/// 集合级 / 文件夹级请求默认值
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RequestDefaults {
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub variables: Vec<Variable>,
    #[serde(default)]
    pub scripts: Option<serde_json::Value>,
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct OpenCollection {
    #[serde(default, rename = "opencollection")]
    pub version: String,
    #[serde(default)]
    pub info: Info,
    #[serde(default)]
    pub request: RequestDefaults,
    #[serde(default)]
    pub docs: Option<serde_json::Value>,
    #[serde(default)]
    pub bundled: Option<bool>,
    #[serde(default)]
    pub extensions: serde_json::Value,
    #[serde(default)]
    pub config: serde_json::Value,
    /// 磁盘文件里通常不出现，保留兼容
    #[serde(default)]
    pub items: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Folder {
    #[serde(default)]
    pub info: Info,
    #[serde(default)]
    pub request: RequestDefaults,
    #[serde(default)]
    pub docs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Environment {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub variables: Vec<Variable>,
    #[serde(default, rename = "extends")]
    pub extends: Option<String>,
    #[serde(default)]
    pub dot_env_file_path: Option<String>,
    #[serde(default, rename = "dotEnvFilePath")]
    pub dot_env_file_path_camel: Option<String>,
}

/// 磁盘上的一个请求条目（路径由文件系统提供，不在 yml 内）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestFile {
    #[serde(default)]
    pub pathname: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub request: HttpRequest,
    /// 前端树节点需要的字段
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 交给前端的集合树节点
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeNode {
    pub uid: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub seq: i64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub pathname: String,
    /// 叶子请求必须没有 items 字段：Bruno 的 isItemARequest 用 `!item.items` 判断请求。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<TreeNode>,
    /// 仅请求节点有 request。文件夹若带了这个键，前端 isItemAFolder 会判成请求，
    /// 因此必须整体省略而不是写成 null；文件夹默认值放 `root`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<serde_json::Value>,
    /// 文件夹默认值（前端读 item.root.request.headers / auth / vars）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    #[serde(default)]
    pub collapsed: bool,
}

impl TreeNode {
    pub fn folder(name: String, pathname: String, filename: String) -> Self {
        Self {
            // 稳定 uid：同一路径每次重载都相同，前端 mergeTreeItems 才能按 uid 保住草稿与标签页
            uid: crate::ops::stable_uid(&pathname),
            name,
            kind: "folder".into(),
            seq: 0,
            tags: vec![],
            filename,
            pathname,
            items: vec![],
            request: None,
            root: None,
            settings: None,
            collapsed: true,
        }
    }
}

/// 21 位字母数字（与 Bruno 的 uidSchema 一致：length(21) + /^[a-zA-Z0-9]*$/）
pub fn new_uid() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let id = uuid::Uuid::new_v4();
    let bytes = id.as_bytes();
    (0..21)
        .map(|i| ALPHABET[bytes[i % bytes.len()] as usize % ALPHABET.len()] as char)
        .collect()
}
