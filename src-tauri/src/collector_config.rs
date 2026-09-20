//! Narrow, reversible routing edits. Credential fields never enter the journal.
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use toml_edit::{value, DocumentMut, Item, Table};

const CONFIG_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RouteJournal {
    pub config_path: PathBuf,
    pub upstream: String,
    pub local_url: String,
    fields: Vec<OwnedField>,
    #[serde(default)]
    created_tables: Vec<Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct OwnedField {
    path: Vec<String>,
    before: Option<String>,
    applied: String,
}

fn read_config(path: &Path) -> Result<(String, DocumentMut), String> {
    if fs::metadata(path).map_err(|_| "无法读取 Codex 配置")?.len() > CONFIG_LIMIT {
        return Err("Codex 配置文件过大".into());
    }
    let text = fs::read_to_string(path).map_err(|_| "无法读取 Codex 配置")?;
    let document = text.parse().map_err(|_| "Codex 配置不是有效的 TOML")?;
    Ok((text, document))
}

fn get<'a>(mut item: &'a Item, path: &[String]) -> Option<&'a Item> {
    for key in path {
        item = item.as_table_like()?.get(key)?;
    }
    Some(item)
}

fn set(item: &mut Item, path: &[String], replacement: Option<Item>) -> Result<(), String> {
    let (key, rest) = path.split_first().ok_or("无效的配置字段")?;
    let table = item
        .as_table_like_mut()
        .ok_or("配置字段的父级不是 TOML 表")?;
    if rest.is_empty() {
        if let Some(mut replacement) = replacement {
            // Leave inline comments in config.toml, never copy them into the journal.
            if let Some(decor) = table
                .get(key)
                .and_then(Item::as_value)
                .map(|v| v.decor().clone())
            {
                if let Some(value) = replacement.as_value_mut() {
                    *value.decor_mut() = decor;
                }
            }
            table.insert(key, replacement);
        } else {
            table.remove(key);
        }
    } else {
        if table.get(key).is_none() {
            if replacement.is_none() {
                return Ok(());
            }
            table.insert(key, Item::Table(Table::new()));
        }
        set(table.get_mut(key).ok_or("缺少配置表")?, rest, replacement)?;
    }
    Ok(())
}

fn parsed_item(text: &str) -> Result<Item, String> {
    let document: DocumentMut = format!("value = {text}\n")
        .parse()
        .map_err(|_| "采集路由备份无效")?;
    Ok(document["value"].clone())
}

fn equivalent(left: &Item, right: &Item) -> bool {
    match (
        left.as_str(),
        right.as_str(),
        left.as_bool(),
        right.as_bool(),
    ) {
        (Some(a), Some(b), _, _) => a == b,
        (_, _, Some(a), Some(b)) => a == b,
        _ => false,
    }
}

pub(crate) fn atomic_write(
    path: &Path,
    bytes: &[u8],
    expected: Option<&str>,
) -> Result<(), String> {
    let parent = path.parent().ok_or("文件路径无效")?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?
        .as_nanos();
    let temporary = parent.join(format!(".wtm-{}-{nonce}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| "无法创建配置临时文件")?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "无法写入配置")?;
        if let Some(expected) = expected {
            if fs::read_to_string(path).ok().as_deref() != Some(expected) {
                return Err("配置正在被其他程序修改，请重试".into());
            }
        }
        fs::rename(&temporary, path).map_err(|_| "无法原子替换配置文件")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

impl RouteJournal {
    /// Build the journal before starting a relay or touching configuration.
    pub fn prepare(path: &Path, port: u16) -> Result<(Self, String, String), String> {
        let config_path = fs::canonicalize(path).map_err(|_| "找不到 Codex config.toml")?;
        let (original, mut document) = read_config(&config_path)?;
        if document.get("profile").is_some() {
            return Err("自动接入暂不支持默认 profile；请使用主 config.toml 或独立采集命令".into());
        }
        let selected = document
            .get("model_provider")
            .and_then(Item::as_str)
            .unwrap_or("openai")
            .to_owned();
        if selected.is_empty()
            || selected.len() > 128
            || !selected
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err("不支持的 provider 名称".into());
        }
        let official = selected == "openai";
        let provider = if official {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "系统时间无效")?
                .as_nanos();
            format!("wtm_monitor_{nonce}")
        } else {
            selected
        };
        let upstream = if official {
            "https://chatgpt.com/backend-api/codex".to_owned()
        } else {
            let entry = document
                .get("model_providers")
                .and_then(|v| v.as_table_like())
                .and_then(|t| t.get(&provider))
                .ok_or("找不到当前自定义 provider")?;
            if entry
                .get("wire_api")
                .and_then(Item::as_str)
                .unwrap_or("responses")
                != "responses"
            {
                return Err("仅支持 Responses provider".into());
            }
            entry
                .get("base_url")
                .and_then(Item::as_str)
                .ok_or("provider 缺少 base_url")?
                .trim_end_matches('/')
                .to_owned()
        };
        let url = tauri::Url::parse(&upstream).map_err(|_| "无效的上游地址")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        {
            return Err(
                "自动接入需要原始 HTTPS 上游地址（不能是本地采集地址或含凭据的 URL）".into(),
            );
        }
        let local_url = format!("http://127.0.0.1:{port}/v1");
        let mut fields = Vec::new();
        let mut created_tables = Vec::new();
        let mut edit = |path: Vec<String>, applied: Item| -> Result<(), String> {
            for length in 1..path.len() {
                if get(document.as_item(), &path[..length]).is_none() {
                    created_tables.push(path[..length].to_vec());
                }
            }
            if let Some(existing) = get(document.as_item(), &path) {
                if (applied.as_str().is_some() && existing.as_str().is_none())
                    || (applied.as_bool().is_some() && existing.as_bool().is_none())
                {
                    return Err("采集相关配置字段的类型无效，未修改配置".into());
                }
            }
            let before = get(document.as_item(), &path).map(|item| {
                if let Some(text) = item.as_str() {
                    value(text).to_string()
                } else {
                    value(item.as_bool().expect("validated scalar field")).to_string()
                }
            });
            fields.push(OwnedField {
                path: path.clone(),
                before,
                applied: applied.to_string(),
            });
            set(document.as_item_mut(), &path, Some(applied))
        };
        edit(
            vec!["features".into(), "enable_request_compression".into()],
            value(false),
        )?;
        let prefix = vec!["model_providers".into(), provider.clone()];
        for (name, item) in [
            ("base_url", value(local_url.clone())),
            ("supports_websockets", value(false)),
        ] {
            let mut path = prefix.clone();
            path.push(name.into());
            edit(path, item)?;
        }
        if official {
            edit(vec!["model_provider".into()], value(provider))?;
            for (name, item) in [
                ("name", value("OpenAI")),
                ("wire_api", value("responses")),
                ("requires_openai_auth", value(true)),
            ] {
                let mut path = prefix.clone();
                path.push(name.into());
                edit(path, item)?;
            }
        }
        Ok((
            Self {
                config_path,
                upstream,
                local_url,
                fields,
                created_tables,
            },
            original,
            document.to_string(),
        ))
    }

    pub fn still_installed(&self) -> bool {
        let Ok((_, document)) = read_config(&self.config_path) else {
            return false;
        };
        self.fields.iter().all(|field| {
            let applied = parsed_item(&field.applied).ok();
            get(document.as_item(), &field.path)
                .zip(applied.as_ref())
                .is_some_and(|(a, b)| equivalent(a, b))
        })
    }

    pub fn restore(&self) -> Result<usize, String> {
        let (original, mut document) = read_config(&self.config_path)?;
        let mut preserved = 0;
        for field in &self.fields {
            let applied = parsed_item(&field.applied)?;
            if get(document.as_item(), &field.path)
                .is_some_and(|current| equivalent(current, &applied))
            {
                let before = field.before.as_deref().map(parsed_item).transpose()?;
                set(document.as_item_mut(), &field.path, before)?;
            } else {
                preserved += 1;
            }
        }
        for path in self.created_tables.iter().rev() {
            if get(document.as_item(), path)
                .and_then(Item::as_table_like)
                .is_some_and(|table| table.is_empty())
            {
                set(document.as_item_mut(), path, None)?;
            }
        }
        let restored = document.to_string();
        if restored != original {
            atomic_write(&self.config_path, restored.as_bytes(), Some(&original))?;
        }
        Ok(preserved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "wtm-route-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn config(&self, text: &str) -> PathBuf {
            let p = self.0.join("config.toml");
            fs::write(&p, text).unwrap();
            p
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    const CONFIG: &str = "# keep this comment\nmodel_provider = 'custom'\nmodel = 'test-model'\n[model_providers.custom]\nname = 'Fixture'\nbase_url = 'https://example.com/v1' # original\nexperimental_bearer_token = 'synthetic-secret'\n";
    #[test]
    fn roundtrip_preserves_auth_and_unrelated_edits_without_journaling_credentials() {
        let fixture = Fixture::new();
        let path = fixture.config(CONFIG);
        let (journal, original, patched) = RouteJournal::prepare(&path, 4319).unwrap();
        assert!(!serde_json::to_string(&journal)
            .unwrap()
            .contains("synthetic-secret"));
        assert!(!serde_json::to_string(&journal)
            .unwrap()
            .contains("# original"));
        atomic_write(&path, patched.as_bytes(), Some(&original)).unwrap();
        assert!(journal.still_installed());
        let edited = patched.replace("test-model", "new-model");
        fs::write(&path, edited).unwrap();
        assert_eq!(journal.restore().unwrap(), 0);
        let result = fs::read_to_string(path).unwrap();
        assert!(result.contains("# keep this comment"));
        assert!(result.contains("# original"));
        assert!(result.contains("synthetic-secret"));
        assert!(result.contains("new-model"));
        assert!(result.contains("https://example.com/v1"));
        assert!(!result.contains("127.0.0.1"));
    }
    #[test]
    fn restore_does_not_overwrite_external_route_edits() {
        let fixture = Fixture::new();
        let path = fixture.config(CONFIG);
        let (journal, original, patched) = RouteJournal::prepare(&path, 4319).unwrap();
        atomic_write(&path, patched.as_bytes(), Some(&original)).unwrap();
        fs::write(
            &path,
            patched.replace("http://127.0.0.1:4319/v1", "https://changed.example/v1"),
        )
        .unwrap();
        assert!(!journal.still_installed());
        assert_eq!(journal.restore().unwrap(), 1);
        assert!(fs::read_to_string(path)
            .unwrap()
            .contains("https://changed.example/v1"));
    }
    #[test]
    fn official_provider_uses_client_oauth_and_restores_selection() {
        let fixture = Fixture::new();
        let path = fixture.config("model='test'\n");
        let (journal, original, patched) = RouteJournal::prepare(&path, 4319).unwrap();
        assert!(patched.contains("requires_openai_auth = true"));
        assert!(!patched.contains("env_key"));
        atomic_write(&path, patched.as_bytes(), Some(&original)).unwrap();
        journal.restore().unwrap();
        let (_, doc) = read_config(&path).unwrap();
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model_providers").is_none());
        assert!(doc.get("features").is_none());
    }
    #[test]
    fn refuses_loopback_and_racing_config_write() {
        let fixture = Fixture::new();
        let path = fixture.config(CONFIG);
        let (_, original, patched) = RouteJournal::prepare(&path, 4319).unwrap();
        fs::write(&path, "model='external-edit'\n").unwrap();
        assert!(atomic_write(&path, patched.as_bytes(), Some(&original)).is_err());
        fs::write(
            &path,
            CONFIG.replace("https://example.com/v1", "http://127.0.0.1:4319/v1"),
        )
        .unwrap();
        assert!(RouteJournal::prepare(&path, 4319).is_err());
    }
}
