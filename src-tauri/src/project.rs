//! Modelo de projeto e persistência (`~/LocalTusk/projects.json`).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DB_USER: &str = "root";
pub const DB_PASSWORD: &str = "root";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Slug único; também é o nome da pasta do projeto.
    pub id: String,
    pub name: String,
    /// Subpasta de `app/` servida pelo Caddy (ex.: `public`). Vazio = raiz.
    pub docroot: String,
    /// Versão major.minor do PHP (ex.: `8.3`). Vazio = a mais nova instalada.
    pub php_version: String,
    /// Versão do MySQL (completa `8.4.9` ou série `8.4`). Vazio = a mais nova instalada.
    #[serde(default)]
    pub db_version: String,
    pub http_port: u16,
    pub db_port: u16,
    pub db_name: String,
    pub created_at: u64,
}

pub fn project_dir(base: &Path, id: &str) -> PathBuf {
    base.join("projects").join(id)
}

fn store_file(base: &Path) -> PathBuf {
    base.join("projects.json")
}

pub fn load(base: &Path) -> Vec<Project> {
    std::fs::read_to_string(store_file(base))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(base: &Path, projects: &[Project]) -> Result<(), String> {
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(projects).map_err(|e| e.to_string())?;
    std::fs::write(store_file(base), json).map_err(|e| e.to_string())
}

pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in name.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "projeto".into() } else { out }
}

/// Domínio do projeto: `<id>.localhost`. Navegadores e o macOS resolvem `*.localhost`
/// para 127.0.0.1 sem nenhuma configuração (e sem precisar de senha de administrador).
pub fn site_host(p: &Project) -> String {
    format!("{}.localhost", p.id)
}

/// Endereço do site: `http://<id>.localhost:<porta>`.
pub fn site_url(p: &Project) -> String {
    format!("http://{}:{}", site_host(p), p.http_port)
}

pub fn unique_id(name: &str, existing: &[Project]) -> String {
    let base = slugify(name);
    let taken: HashSet<&str> = existing.iter().map(|p| p.id.as_str()).collect();
    if !taken.contains(base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken.contains(c.as_str()))
        .unwrap()
}

pub fn port_is_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// Primeira porta livre a partir de `start`, ignorando as já reservadas por projetos.
pub fn free_port(start: u16, reserved: &HashSet<u16>) -> Option<u16> {
    (start..start.saturating_add(2000)).find(|p| !reserved.contains(p) && port_is_free(*p))
}

/// `n` portas livres e distintas (para serviços que não precisam de porta fixa).
pub fn ephemeral_ports(n: usize) -> Option<Vec<u16>> {
    // Mantém todos os sockets abertos até o fim para o SO não repetir a mesma porta.
    let listeners: Vec<TcpListener> = (0..n)
        .map(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok())
        .collect::<Option<_>>()?;
    listeners
        .iter()
        .map(|l| l.local_addr().ok().map(|a| a.port()))
        .collect()
}

pub fn db_name_for(id: &str) -> String {
    id.replace('-', "_")
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn reserved_ports(projects: &[Project]) -> HashSet<u16> {
    projects
        .iter()
        .flat_map(|p| [p.http_port, p.db_port])
        .collect()
}

/// Pasta de dados atual e a do nome antigo do app (LocalPHP).
const DATA_DIR: &str = "LocalTusk";
const LEGACY_DATA_DIR: &str = "LocalPHP";

/// Pasta de dados do app dentro de `home`. Se só existir a pasta do nome antigo
/// (`~/LocalPHP`), ela é renomeada para `~/LocalTusk` — projetos, bancos, backups e
/// blueprints seguem intactos. Se o rename falhar, continua usando a pasta antiga.
pub fn resolve_base(home: &Path) -> PathBuf {
    let base = home.join(DATA_DIR);
    let legacy = home.join(LEGACY_DATA_DIR);
    if !base.exists() && legacy.is_dir() && std::fs::rename(&legacy, &base).is_err() {
        return legacy;
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("localtusk-test-home-{tag}-{}", now()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn migra_a_pasta_do_nome_antigo() {
        let home = tmp_home("migra");
        std::fs::create_dir_all(home.join("LocalPHP/projects/meu-site")).unwrap();
        std::fs::write(home.join("LocalPHP/projects.json"), "[]").unwrap();
        let base = resolve_base(&home);
        assert_eq!(base, home.join("LocalTusk"));
        assert!(base.join("projects/meu-site").is_dir());
        assert!(base.join("projects.json").is_file());
        assert!(!home.join("LocalPHP").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn nao_mexe_se_a_pasta_nova_ja_existe() {
        let home = tmp_home("nova");
        std::fs::create_dir_all(home.join("LocalPHP")).unwrap();
        std::fs::create_dir_all(home.join("LocalTusk")).unwrap();
        let base = resolve_base(&home);
        assert_eq!(base, home.join("LocalTusk"));
        assert!(home.join("LocalPHP").is_dir());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn instalacao_nova_usa_a_pasta_nova() {
        let home = tmp_home("fresh");
        assert_eq!(resolve_base(&home), home.join("LocalTusk"));
        assert!(!home.join("LocalPHP").exists());
        let _ = std::fs::remove_dir_all(&home);
    }
}
