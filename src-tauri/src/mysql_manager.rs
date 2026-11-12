//! Gerenciador de versões do MySQL.
//!
//! Baixa os tarballs oficiais do MySQL Community Server (macOS), do CDN da Oracle, e guarda
//! só o necessário em `~/LocalTusk/bin/mysql/<versão>/` (`bin/mysqld`, `lib/`, `share/`).
//!
//! O catálogo de versões é uma lista **embutida** no app (`resources/mysql-versions.json`),
//! gerada por `scripts/update_mysql_versions.py`. O app não sonda servidores em tempo de
//! execução: o CDN da Oracle bloqueia rajadas de requisições, e assim o catálogo é instantâneo
//! e funciona offline.

use crate::download::{self, InstallProgress};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = include_str!("../resources/mysql-versions.json");
const CDN: &str = "https://cdn.mysql.com";

pub fn mysql_root(base: &Path) -> PathBuf {
    base.join("bin").join("mysql")
}

pub fn install_dir(base: &Path, version: &str) -> PathBuf {
    mysql_root(base).join(version)
}

/// Aceita só `X.Y.Z` numérico (impede path traversal).
pub fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
}

fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn series_of(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

fn arch() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("arm64"),
        ("macos", "x86_64") => Some("x86_64"),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Manifest {
    versions: Vec<ManifestVersion>,
}

#[derive(Deserialize)]
struct ManifestVersion {
    version: String,
    builds: HashMap<String, Build>,
}

#[derive(Deserialize, Clone)]
struct Build {
    /// Pasta no CDN: `Downloads/MySQL-8.4` (versão atual) ou `archives/mysql-8.4` (anteriores).
    path: String,
    /// Rótulo do macOS usado no nome do arquivo (`macos15`, `macos14`…).
    label: String,
    bytes: u64,
}

fn manifest() -> Vec<ManifestVersion> {
    serde_json::from_str::<Manifest>(MANIFEST)
        .map(|m| m.versions)
        .unwrap_or_default()
}

/// URLs candidatas para baixar uma versão. Quando uma versão deixa de ser a atual da série o
/// Oracle a move de `Downloads/` para `archives/`, então a segunda opção cobre esse caso.
fn urls_for(version: &str, build: &Build, arch: &str) -> Vec<String> {
    let series = series_of(version);
    let file = format!("mysql-{version}-{}-{arch}.tar.gz", build.label);
    let mut paths = vec![build.path.clone()];
    for alt in [format!("Downloads/MySQL-{series}"), format!("archives/mysql-{series}")] {
        if !paths.contains(&alt) {
            paths.push(alt);
        }
    }
    paths.into_iter().map(|p| format!("{CDN}/{p}/{file}")).collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MysqlRelease {
    pub version: String,
    pub series: String,
    /// Tamanho do download (ex.: `160 MB`).
    pub size: String,
    pub installed: bool,
    /// `true` se for o patch mais novo da série.
    pub latest: bool,
}

/// Versões disponíveis para download nesta máquina, da mais nova para a mais antiga.
pub fn catalog(base: &Path) -> Result<Vec<MysqlRelease>, String> {
    let arch = arch().ok_or("O download do MySQL só está disponível para macOS por enquanto.")?;
    let installed = managed_versions(base);
    let mut versions: Vec<(String, u64)> = manifest()
        .into_iter()
        .filter_map(|v| v.builds.get(arch).map(|b| (v.version, b.bytes)))
        .collect();
    versions.sort_by(|a, b| version_key(&b.0).cmp(&version_key(&a.0)));

    let mut seen = std::collections::HashSet::new();
    Ok(versions
        .into_iter()
        .map(|(version, bytes)| MysqlRelease {
            series: series_of(&version),
            size: format!("{} MB", bytes / 1024 / 1024),
            installed: installed.contains(&version),
            latest: seen.insert(series_of(&version)),
            version,
        })
        .collect())
}

/// Caminhos que valem a pena guardar (o tarball completo tem centenas de MB de testes e extras).
fn keep(rel: &Path) -> bool {
    let s = rel.to_string_lossy();
    // `bin/*.dylib`: o mysqld carrega libprotobuf-lite via @loader_path (na própria pasta bin/).
    let bin_lib = s.starts_with("bin/") && !s[4..].contains('/') && (s.ends_with(".dylib") || s.contains(".so"));
    bin_lib
        || s.starts_with("lib/")
        || s.starts_with("share/")
        || matches!(
            s.as_ref(),
            "bin/mysqld" | "bin/mysql" | "bin/mysqldump" | "bin/mysqladmin"
        )
}

/// Baixa, extrai (só o necessário) e valida uma versão. Bloqueante.
pub fn install(base: &Path, version: &str, progress: &dyn Fn(InstallProgress)) -> Result<(), String> {
    if !valid_version(version) {
        return Err("versão inválida".into());
    }
    let arch = arch().ok_or("O download do MySQL só está disponível para macOS por enquanto.")?;
    let final_dir = install_dir(base, version);
    if final_dir.join("bin").join("mysqld").is_file() {
        return Ok(());
    }

    let id = format!("mysql-{version}");
    let emit = |stage: &str, downloaded: u64, total: u64| {
        progress(InstallProgress {
            id: id.clone(),
            stage: stage.to_string(),
            downloaded,
            total,
            message: None,
        })
    };

    let build = manifest()
        .into_iter()
        .find(|v| v.version == version)
        .and_then(|v| v.builds.get(arch).cloned())
        .ok_or_else(|| format!("MySQL {version} não está disponível para este Mac."))?;
    let urls = urls_for(version, &build, arch);

    let root = mysql_root(base);
    let tmp_dir = root.join(format!(".tmp-{version}"));
    let archive = root.join(format!(".download-{version}.tar.gz"));
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;

    let result = (|| -> Result<(), String> {
        let mut last_err = String::new();
        let mut ok = false;
        for url in &urls {
            match download::download_to(url, &archive, &mut |d, t| emit("download", d, t)) {
                Ok(()) => {
                    ok = true;
                    break;
                }
                // 404: a versão pode ter mudado de pasta no CDN; tenta a próxima.
                Err(e) if e.contains("404") => last_err = e,
                Err(e) => return Err(format!("MySQL {version}: {e}")),
            }
        }
        if !ok {
            return Err(format!("MySQL {version}: {last_err}"));
        }

        emit("extract", 0, 0);
        fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
        let file = File::open(&archive).map_err(|e| e.to_string())?;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| format!("falha ao extrair: {e}"))?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            // Remove a pasta raiz (`mysql-8.4.9-macos15-arm64/`) e rejeita `..`/caminhos absolutos.
            let comps: Vec<Component> = path.components().skip(1).collect();
            if comps.is_empty() || !comps.iter().all(|c| matches!(c, Component::Normal(_))) {
                continue;
            }
            let rel: PathBuf = comps.iter().collect();
            if !keep(&rel) {
                continue;
            }
            let out = tmp_dir.join(&rel);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            entry
                .unpack(&out)
                .map_err(|e| format!("falha ao extrair {}: {e}", rel.display()))?;
        }

        // Verificação: o binário precisa rodar e reportar a versão pedida.
        emit("verify", 0, 0);
        let mysqld = tmp_dir.join("bin").join("mysqld");
        if !mysqld.is_file() {
            return Err("o pacote do MySQL não contém o mysqld".into());
        }
        let out = Command::new(&mysqld)
            .arg("--version")
            .output()
            .map_err(|e| format!("o mysqld baixado não executa: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        if !text.contains(&format!("Ver {version}")) {
            let err = String::from_utf8_lossy(&out.stderr);
            let detail = text.lines().chain(err.lines()).find(|l| !l.trim().is_empty()).unwrap_or("sem saída");
            return Err(format!("verificação falhou ({}): {detail}", out.status));
        }

        let _ = fs::remove_dir_all(&final_dir);
        fs::rename(&tmp_dir, &final_dir).map_err(|e| e.to_string())?;
        Ok(())
    })();

    let _ = fs::remove_file(&archive);
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp_dir);
    }
    result
}

pub fn uninstall(base: &Path, version: &str) -> Result<(), String> {
    if !valid_version(version) {
        return Err("versão inválida".into());
    }
    let dir = install_dir(base, version);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ------------------------------------------------------------------ instalações locais

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MysqlInstall {
    pub full_version: String,
    pub series: String,
    pub mysqld: PathBuf,
    /// Instalado pelo LocalTusk (true) ou vindo do sistema, ex.: Homebrew (false).
    pub managed: bool,
}

pub fn managed_versions(base: &Path) -> Vec<String> {
    managed_installs(base).into_iter().map(|i| i.full_version).collect()
}

fn managed_installs(base: &Path) -> Vec<MysqlInstall> {
    let Ok(rd) = fs::read_dir(mysql_root(base)) else {
        return vec![];
    };
    rd.flatten()
        .filter_map(|e| {
            let version = e.file_name().to_string_lossy().to_string();
            let mysqld = e.path().join("bin").join("mysqld");
            (valid_version(&version) && mysqld.is_file()).then(|| MysqlInstall {
                series: series_of(&version),
                full_version: version,
                mysqld,
                managed: true,
            })
        })
        .collect()
}

/// `mysqld  Ver 9.7.1 for macos26.6 on arm64 (Homebrew)` -> `9.7.1`
fn mysqld_version(bin: &Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let after = text.split("Ver ").nth(1)?;
    let v: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    valid_version(&v).then_some(v)
}

fn system_installs() -> Vec<MysqlInstall> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    let mut candidates: Vec<PathBuf> = ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/local/sbin"]
        .iter()
        .map(|d| Path::new(d).join("mysqld"))
        .collect();
    for prefix in ["/opt/homebrew", "/usr/local"] {
        candidates.push(Path::new(prefix).join("opt/mysql/bin/mysqld"));
        if let Ok(rd) = fs::read_dir(Path::new(prefix).join("opt")) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with("mysql@") {
                    candidates.push(e.path().join("bin/mysqld"));
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).filter(|d| d.is_absolute()).map(|d| d.join("mysqld")));
    }
    for c in candidates {
        let Ok(real) = fs::canonicalize(&c) else { continue };
        if !seen.insert(real) {
            continue;
        }
        if let Some(v) = mysqld_version(&c) {
            out.push(MysqlInstall {
                series: series_of(&v),
                full_version: v,
                mysqld: c,
                managed: false,
            });
        }
    }
    out
}

/// Todas as instalações do MySQL, da mais nova para a mais antiga (a gerenciada vence a do sistema).
pub fn installs(base: &Path) -> Vec<MysqlInstall> {
    let mut all = managed_installs(base);
    for sys in system_installs() {
        if !all.iter().any(|i| i.full_version == sys.full_version) {
            all.push(sys);
        }
    }
    all.sort_by(|a, b| version_key(&b.full_version).cmp(&version_key(&a.full_version)));
    all
}

/// Escolhe o mysqld para uma versão pedida (completa `8.4.9` ou série `8.4`).
/// Sem correspondência usa o mais novo instalado.
pub fn pick(base: &Path, wanted: &str) -> Option<MysqlInstall> {
    let all = installs(base);
    all.iter()
        .find(|i| i.full_version == wanted)
        .or_else(|| all.iter().find(|i| i.series == wanted))
        .cloned()
        .or_else(|| all.into_iter().next())
}
