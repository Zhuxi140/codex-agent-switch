//! 本机 Responses 入口；上游真实凭据只在 CAS 进程中读取。

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::Duration;

use cas_secret_store::{CredentialId, read as read_secret};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::redirect::Policy;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use uuid::Uuid;

use crate::chat_compat::{chat_endpoint, chat_to_responses, responses_to_chat, responses_to_sse};

const LISTEN_ADDR: &str = "127.0.0.1:43187";
const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) fn provider_base_url(provider_id: &str) -> String {
    format!("http://{LISTEN_ADDR}/providers/{provider_id}/")
}

pub(crate) fn start(database_path: PathBuf) -> io::Result<()> {
    let server = Server::http(LISTEN_ADDR).map_err(io::Error::other)?;
    thread::Builder::new()
        .name("cas-chat-gateway".to_owned())
        .spawn(move || {
            while let Ok(request) = server.recv() {
                let database_path = database_path.clone();
                let _ = thread::Builder::new()
                    .name("cas-chat-request".to_owned())
                    .spawn(move || handle(request, &database_path));
            }
        })?;
    Ok(())
}

fn handle(mut request: Request, database_path: &Path) {
    let reply = process(&mut request, database_path);
    let content_type = Header::from_bytes("Content-Type", reply.content_type).unwrap();
    let response = Response::from_data(reply.body)
        .with_status_code(StatusCode(reply.status))
        .with_header(content_type);
    let _ = request.respond(response);
}

struct GatewayReply {
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
}

impl GatewayReply {
    fn error(status: u16, message: &'static str) -> Self {
        Self {
            status,
            body: serde_json::to_vec(
                &json!({"error":{"message":message,"type":"invalid_request_error"}}),
            )
            .expect("static error serializes"),
            content_type: "application/json",
        }
    }
}

fn process(request: &mut Request, database_path: &Path) -> GatewayReply {
    if request.method() != &Method::Post {
        return GatewayReply::error(405, "仅支持 POST");
    }
    let Some(provider_id) = parse_provider_path(request.url()) else {
        return GatewayReply::error(404, "未知 Responses 路径");
    };
    let connection =
        match Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(connection) => connection,
            Err(_) => return GatewayReply::error(503, "CAS 数据库不可用"),
        };
    let provider = connection
        .query_row(
            "SELECT p.base_url, c.id FROM providers p
             JOIN credentials c ON c.provider_id = p.id AND c.credential_key = 'primary'
             WHERE p.id = ?1 AND p.enabled = 1 AND p.protocol = 'CHAT_COMPLETIONS'",
            [&provider_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional();
    let (base_url, credential_id) = match provider {
        Ok(Some(provider)) => provider,
        Ok(None) => return GatewayReply::error(404, "Chat Completions Provider 不存在或未启用"),
        Err(_) => return GatewayReply::error(503, "CAS 数据库查询失败"),
    };
    let authorized = request.headers().iter().any(|header| {
        header.field.equiv("Authorization")
            && header.value.as_str() == format!("Bearer cas-gateway-{credential_id}")
    });
    if !authorized {
        return GatewayReply::error(401, "本机网关认证失败");
    }
    let mut request_bytes = Vec::new();
    if request
        .as_reader()
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut request_bytes)
        .is_err()
        || request_bytes.len() as u64 > MAX_BODY_BYTES
    {
        return GatewayReply::error(413, "Responses 请求过大或无法读取");
    }
    let request_json = match serde_json::from_slice::<Value>(&request_bytes) {
        Ok(value) => value,
        Err(_) => return GatewayReply::error(400, "Responses 请求不是有效 JSON"),
    };
    let prepared = match responses_to_chat(&request_json) {
        Ok(prepared) => prepared,
        Err(_) => return GatewayReply::error(422, "该 Responses 请求包含尚未支持的字段或内容"),
    };
    let endpoint = match chat_endpoint(&base_url) {
        Ok(endpoint) => endpoint,
        Err(_) => return GatewayReply::error(502, "上游 Chat Completions 地址无效"),
    };
    let credential_id = match CredentialId::from_str(&credential_id) {
        Ok(id) => id,
        Err(_) => return GatewayReply::error(503, "Credential 引用无效"),
    };
    let secret = match read_secret(credential_id) {
        Ok(secret) => secret,
        Err(_) => return GatewayReply::error(503, "无法读取上游 Credential"),
    };
    let mut bearer = Vec::with_capacity(7 + secret.expose().len());
    bearer.extend_from_slice(b"Bearer ");
    bearer.extend_from_slice(secret.expose());
    let authorization = HeaderValue::from_bytes(&bearer);
    bearer.fill(0);
    let mut authorization = match authorization {
        Ok(value) => value,
        Err(_) => return GatewayReply::error(503, "上游 Credential 无效"),
    };
    authorization.set_sensitive(true);
    let client = match Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return GatewayReply::error(503, "无法初始化上游连接"),
    };
    let mut upstream = match client
        .post(endpoint)
        .header(AUTHORIZATION, authorization)
        .json(&prepared.body)
        .send()
    {
        Ok(response) => response,
        Err(_) => return GatewayReply::error(502, "上游 Chat Completions 请求失败"),
    };
    let status = upstream.status();
    let mut bytes = Vec::new();
    if upstream
        .by_ref()
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_BODY_BYTES
    {
        return GatewayReply::error(502, "上游响应过大或无法读取");
    }
    if !status.is_success() {
        return if serde_json::from_slice::<Value>(&bytes).is_ok() {
            GatewayReply {
                status: status.as_u16(),
                body: bytes,
                content_type: "application/json",
            }
        } else {
            GatewayReply::error(502, "上游返回非 JSON 错误")
        };
    }
    let response = match serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|completion| chat_to_responses(&completion).ok())
    {
        Some(response) => response,
        None => return GatewayReply::error(502, "上游结果无法转换为 Responses"),
    };
    if prepared.wants_stream {
        GatewayReply {
            status: 200,
            body: responses_to_sse(&response),
            content_type: "text/event-stream",
        }
    } else {
        GatewayReply {
            status: 200,
            body: serde_json::to_vec(&response).expect("Responses serializes"),
            content_type: "application/json",
        }
    }
}

fn parse_provider_path(path: &str) -> Option<String> {
    let id = path
        .strip_prefix("/providers/")?
        .strip_suffix("/responses")?
        .trim_end_matches('/');
    let id = Uuid::parse_str(id).ok()?;
    Some(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_provider_responses_route_is_accepted() {
        let id = Uuid::new_v4().to_string();
        assert_eq!(
            provider_base_url(&id),
            format!("http://127.0.0.1:43187/providers/{id}/")
        );
        assert_eq!(
            parse_provider_path(&format!("/providers/{id}/responses")),
            Some(id.clone())
        );
        assert_eq!(
            parse_provider_path(&format!("/providers/{id}//responses")),
            Some(id.clone())
        );
        assert_eq!(
            parse_provider_path(&format!("/providers/{id}/chat/completions")),
            None
        );
        assert_eq!(parse_provider_path("/providers/not-an-id/responses"), None);
    }
}

#[cfg(test)]
mod e2e;
