//! gRPC server reflection 客户端。
//!
//! 走标准的 bidi 流 `grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo`：
//! 先 `list_services`，再对每个 service 发一次 `file_containing_symbol`，
//! 把返回的 FileDescriptorProto 拼成 FileDescriptorSet 交给 prost-reflect。
//! v1alpha 不可用时回落到 v1（新服务器只实现 v1 的情况）。

use crate::grpc_codec::ProstCodec;
use anyhow::Context;
use prost::Message as ProstMessage;
use prost_reflect::DescriptorPool;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Endpoint;
use tonic::Request;

// ------------------------------- 协议消息 -------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerReflectionRequest {
    #[prost(string, tag = "1")]
    pub host: String,
    #[prost(oneof = "MessageRequest", tags = "3, 4, 5, 6, 7")]
    pub message_request: Option<MessageRequest>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum MessageRequest {
    #[prost(string, tag = "3")]
    FileByFilename(String),
    #[prost(string, tag = "4")]
    FileContainingSymbol(String),
    #[prost(message, tag = "5")]
    FileContainingExtension(ExtensionRequest),
    #[prost(string, tag = "6")]
    AllExtensionNumbersOfType(String),
    #[prost(string, tag = "7")]
    ListServices(String),
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExtensionRequest {
    #[prost(string, tag = "1")]
    pub containing_type: String,
    #[prost(int32, tag = "2")]
    pub extension_number: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerReflectionResponse {
    #[prost(string, tag = "1")]
    pub valid_host: String,
    #[prost(message, tag = "2")]
    pub original_request: Option<ServerReflectionRequest>,
    #[prost(oneof = "MessageResponse", tags = "4, 5, 6, 7")]
    pub message_response: Option<MessageResponse>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum MessageResponse {
    #[prost(message, tag = "4")]
    FileDescriptorResponse(FileDescriptorResponse),
    #[prost(message, tag = "5")]
    AllExtensionNumbersResponse(ExtensionNumberResponse),
    #[prost(message, tag = "6")]
    ListServicesResponse(ListServiceResponse),
    #[prost(message, tag = "7")]
    ErrorResponse(ErrorResponse),
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct FileDescriptorResponse {
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub file_descriptor_proto: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExtensionNumberResponse {
    #[prost(string, tag = "1")]
    pub base_type_name: String,
    #[prost(int32, repeated, tag = "2")]
    pub extension_number: Vec<i32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ListServiceResponse {
    #[prost(message, repeated, tag = "1")]
    pub service: Vec<ServiceResponse>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServiceResponse {
    #[prost(string, tag = "1")]
    pub name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ErrorResponse {
    #[prost(int32, tag = "1")]
    pub error_code: i32,
    #[prost(string, tag = "2")]
    pub error_message: String,
}

// ------------------------------- 客户端 -------------------------------

const V1ALPHA: &str = "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo";
const V1: &str = "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo";

/// 通过 reflection 拉取描述符池。`endpoint` 必须已规范化（带 http/https）。
pub async fn load_pool(
    endpoint: String,
    metadata: Vec<(String, String)>,
    timeout: Duration,
) -> anyhow::Result<DescriptorPool> {
    let attempt = tokio::time::timeout(timeout, load_pool_via(endpoint.clone(), metadata.clone(), V1ALPHA)).await;
    match attempt {
        Ok(Ok(pool)) => return Ok(pool),
        Ok(Err(e)) => {
            // 有可能是「服务端只实现 v1」，也可能是真的没有 reflection，试一次 v1
            let second = tokio::time::timeout(timeout, load_pool_via(endpoint.clone(), metadata, V1)).await;
            match second {
                Ok(Ok(pool)) => return Ok(pool),
                Ok(Err(e2)) => anyhow::bail!("server reflection 不可用：{}（v1 回退同样失败：{e2}）", short(&e)),
                Err(_) => anyhow::bail!("server reflection 不可用：{}", short(&e)),
            }
        }
        Err(_) => anyhow::bail!("server reflection 超时（{}s）", timeout.as_secs()),
    }
}

fn short(e: &anyhow::Error) -> String {
    e.to_string()
}

async fn load_pool_via(endpoint: String, metadata: Vec<(String, String)>, path: &'static str) -> anyhow::Result<DescriptorPool> {
    let channel = Endpoint::from_shared(endpoint.clone())
        .with_context(|| format!("无效的 gRPC 地址：{endpoint}"))?
        .connect_timeout(Duration::from_secs(10))
        .connect()
        .await
        .with_context(|| format!("连接 {endpoint} 失败"))?;

    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await.map_err(|e| anyhow::anyhow!("通道未就绪：{e}"))?;

    let (tx, rx) = mpsc::channel::<ServerReflectionRequest>(8);
    let mut request = Request::new(ReceiverStream::new(rx));
    for (k, v) in &metadata {
        if let (Ok(key), Ok(value)) = (
            tonic::metadata::MetadataKey::from_bytes(k.to_ascii_lowercase().as_bytes()),
            tonic::metadata::MetadataValue::try_from(v.as_str()),
        ) {
            request.metadata_mut().insert(key, value);
        }
    }

    let response = client
        .streaming(request, path.parse()?, ProstCodec::<ServerReflectionRequest, ServerReflectionResponse>::default())
        .await
        .map_err(|s| anyhow::anyhow!("打开 reflection 流失败：{} {}", s.code() as i32, s.message()))?;
    let mut stream = response.into_inner();

    // 1) list_services
    tx.send(ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::ListServices(String::new())),
    })
    .await?;
    let first = stream
        .message()
        .await?
        .ok_or_else(|| anyhow::anyhow!("reflection 流提前结束"))?;

    let services = match first.message_response {
        Some(MessageResponse::ListServicesResponse(list)) => list.service.into_iter().map(|s| s.name).collect::<Vec<_>>(),
        Some(MessageResponse::ErrorResponse(err)) => {
            anyhow::bail!("list_services 返回错误 {}：{}", err.error_code, err.error_message)
        }
        _ => Vec::new(),
    };

    // 2) 每个 service 拿一次描述符
    let mut descriptors: Vec<Vec<u8>> = Vec::new();
    for name in &services {
        tx.send(ServerReflectionRequest {
            host: String::new(),
            message_request: Some(MessageRequest::FileContainingSymbol(name.clone())),
        })
        .await?;
        let Some(msg) = stream.message().await? else { break };
        match msg.message_response {
            Some(MessageResponse::FileDescriptorResponse(fd)) => descriptors.extend(fd.file_descriptor_proto),
            Some(MessageResponse::ErrorResponse(err)) => {
                // 单个 service 失败不影响其它，继续
                let _ = (err.error_code, err.error_message);
            }
            _ => {}
        }
    }
    drop(tx);

    if descriptors.is_empty() {
        anyhow::bail!("server reflection 没有返回任何描述符（服务端可能未开启 reflection）");
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut files = Vec::new();
    for bytes in descriptors {
        let fdp = prost_types::FileDescriptorProto::decode(&bytes[..])
            .map_err(|e| anyhow::anyhow!("解析 FileDescriptorProto 失败：{e}"))?;
        let key = fdp.name.clone().unwrap_or_default();
        if seen.insert(key) {
            files.push(fdp);
        }
    }
    let set = prost_types::FileDescriptorSet { file: files };
    DescriptorPool::from_file_descriptor_set(set).map_err(|e| anyhow::anyhow!("构建描述符池失败：{e}"))
}

/// 只列服务名（诊断用）
pub async fn list_services(endpoint: String, metadata: Vec<(String, String)>) -> anyhow::Result<Vec<String>> {
    let pool = load_pool(endpoint, metadata, Duration::from_secs(15)).await?;
    Ok(pool.services().map(|s| s.full_name().to_string()).collect())
}
