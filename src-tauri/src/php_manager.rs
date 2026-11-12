//! Gerenciador de versões do PHP.
//!
//! Baixa binários estáticos do PHP-FPM (projeto static-php-cli, pacote `bulk`, que inclui
//! mysqli, intl, opcache, imagick, sodium, etc.) e os guarda em
//! `~/LocalTusk/bin/php/<versão>/php-fpm`. O usuário nunca precisa escolher onde instalar.

use crate::download::{self, InstallProgress};
use crate::tools;
use serde::Serialize;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

const BASE_URL: &str = "https://dl.static-php.dev/static-php-cli/bulk";

/// Sufixo de plataforma usado nos nomes dos arquivos do static-php-cli.
fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

pub fn php_root(base: &Path) -> PathBuf {
    base.join("bin").join("php")
}

pub fn install_dir(base: &Path, version: &str) -> PathBuf {
    php_root(base).join(version)
}

/// Aceita só `X.Y.Z` numérico, o que também impede path traversal.
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpRelease {
    pub version: String,
    pub minor: String,
    /// Tamanho do download, como informado pelo servidor (ex.: `33.9M`).
    pub size: String,
    pub installed: bool,
    /// `true` se for o patch mais novo da sua série (8.3.x).
    pub latest: bool,
}

/// Consulta o servidor e devolve todas as versões disponíveis para esta máquina.
pub fn catalog(base: &Path) -> Result<Vec<PhpRelease>, String> {
    let platform = platform().ok_or("Esta plataforma ainda não é suportada para download do PHP.")?;
    let suffix = format!("-fpm-{platform}.tar.gz");

    let body = download::get_string(&format!("{BASE_URL}/?format=json"))
        .map_err(|e| format!("Não foi possível consultar as versões do PHP (sem internet?): {e}"))?;
    let listing: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;

    let installed: Vec<String> = fs::read_dir(php_root(base))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join("php-fpm").is_file())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();

    let mut releases: Vec<PhpRelease> = listing
        .as_array()
        .ok_or("resposta inesperada do servidor")?
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let version = name.strip_prefix("php-")?.strip_suffix(suffix.as_str())?;
            if !valid_version(version) {
                return None;
            }
            let minor = version.split('.').take(2).collect::<Vec<_>>().join(".");
            Some(PhpRelease {
                version: version.to_string(),
                minor,
                size: item.get("size").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                installed: installed.iter().any(|i| i == version),
                latest: false,
            })
        })
        .collect();

    releases.sort_by(|a, b| version_key(&b.version).cmp(&version_key(&a.version)));
    let mut seen = std::collections::HashSet::new();
    for r in releases.iter_mut() {
        r.latest = seen.insert(r.minor.clone());
    }
    Ok(releases)
}

/// Caminho do PHP de linha de comando (`php`) de uma versão gerenciada, se instalado.
pub fn cli_bin(base: &Path, version: &str) -> Option<PathBuf> {
    let p = install_dir(base, version).join("php");
    p.is_file().then_some(p)
}

/// Extrai um pacote do static-php-cli em `dest`, deixa o binário executável e confere
/// que ele roda e reporta a versão pedida.
fn unpack_binary(archive: &Path, dest: &Path, bin_name: &str, version: &str) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let file = File::open(archive).map_err(|e| e.to_string())?;
    tar::Archive::new(flate2::read::GzDecoder::new(file))
        .unpack(dest)
        .map_err(|e| format!("falha ao extrair: {e}"))?;
    let bin = dest.join(bin_name);
    if !bin.is_file() {
        return Err(format!("o pacote baixado não contém o {bin_name}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let out = Command::new(&bin)
        .arg("-v")
        .output()
        .map_err(|e| format!("o binário baixado não executa: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !text.contains(&format!("PHP {version}")) {
        return Err(format!("verificação falhou: {}", text.lines().next().unwrap_or("sem saída")));
    }
    Ok(())
}

/// Baixa, extrai e valida uma versão (PHP-FPM e PHP CLI). Bloqueante; `progress` é
/// chamado durante o processo. O CLI é opcional: se não existir para esta versão,
/// o PHP-FPM ainda é instalado.
pub fn install(
    base: &Path,
    version: &str,
    progress: &dyn Fn(InstallProgress),
) -> Result<(), String> {
    if !valid_version(version) {
        return Err("versão inválida".into());
    }
    let platform = platform().ok_or("Plataforma não suportada.")?;
    let final_dir = install_dir(base, version);
    if final_dir.join("php-fpm").is_file() {
        return Ok(());
    }

    let root = php_root(base);
    let tmp_dir = root.join(format!(".tmp-{version}"));
    let fpm_archive = root.join(format!(".download-{version}-fpm.tar.gz"));
    let cli_archive = root.join(format!(".download-{version}-cli.tar.gz"));
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;

    let emit = |stage: &str, downloaded: u64, total: u64| {
        progress(InstallProgress {
            id: version.to_string(),
            stage: stage.to_string(),
            downloaded,
            total,
            message: None,
        })
    };

    let result = (|| -> Result<(), String> {
        // 1) Downloads. O progresso soma os dois arquivos (têm tamanho parecido).
        let url = |flavor: &str| format!("{BASE_URL}/php-{version}-{flavor}-{platform}.tar.gz");
        let mut fpm_total = 0u64;
        download::download_to(&url("fpm"), &fpm_archive, &mut |d, t| {
            fpm_total = t;
            emit("download", d, t * 2)
        })
        .map_err(|e| format!("PHP {version}: {e}"))?;
        let cli_ok = download::download_to(&url("cli"), &cli_archive, &mut |d, t| {
            emit("download", fpm_total + d, fpm_total + t)
        })
        .is_ok();

        // 2) Extração e verificação: os binários precisam rodar e reportar a versão pedida.
        emit("extract", 0, 0);
        unpack_binary(&fpm_archive, &tmp_dir, "php-fpm", version)?;
        emit("verify", 0, 0);
        if cli_ok {
            // Sem o CLI o PHP-FPM continua válido; `ensure_cli` tenta de novo quando precisar.
            if unpack_binary(&cli_archive, &tmp_dir, "php", version).is_err() {
                let _ = fs::remove_file(tmp_dir.join("php"));
            }
        }

        // 3) Publica de forma atômica.
        let _ = fs::remove_dir_all(&final_dir);
        fs::rename(&tmp_dir, &final_dir).map_err(|e| e.to_string())?;
        Ok(())
    })();

    let _ = fs::remove_file(&fpm_archive);
    let _ = fs::remove_file(&cli_archive);
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp_dir);
    }
    result
}

/// Garante o PHP CLI de uma versão instalada pelo app (versões instaladas antes de o
/// app baixar o CLI não o têm). Bloqueante.
pub fn ensure_cli(base: &Path, version: &str, progress: &dyn Fn(InstallProgress)) -> Result<PathBuf, String> {
    if !valid_version(version) {
        return Err("versão inválida".into());
    }
    if let Some(p) = cli_bin(base, version) {
        return Ok(p);
    }
    let dir = install_dir(base, version);
    if !dir.join("php-fpm").is_file() {
        return Err(format!("o PHP {version} não está instalado"));
    }
    let platform = platform().ok_or("Plataforma não suportada.")?;
    let archive = php_root(base).join(format!(".download-{version}-cli.tar.gz"));
    let tmp = php_root(base).join(format!(".tmp-{version}-cli"));
    let _ = fs::remove_dir_all(&tmp);

    let emit = |stage: &str, downloaded: u64, total: u64| {
        progress(InstallProgress {
            id: version.to_string(),
            stage: stage.to_string(),
            downloaded,
            total,
            message: Some("Baixando o PHP CLI…".into()),
        })
    };
    let result = (|| -> Result<(), String> {
        let url = format!("{BASE_URL}/php-{version}-cli-{platform}.tar.gz");
        download::download_to(&url, &archive, &mut |d, t| emit("download", d, t))
            .map_err(|e| format!("PHP CLI {version}: {e}"))?;
        emit("extract", 0, 0);
        unpack_binary(&archive, &tmp, "php", version)?;
        fs::rename(tmp.join("php"), dir.join("php")).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(&archive);
    let _ = fs::remove_dir_all(&tmp);
    result?;
    cli_bin(base, version).ok_or_else(|| "o PHP CLI não foi instalado".into())
}

pub fn uninstall(base: &Path, version: &str) -> Result<(), String> {
    if !valid_version(version) {
        return Err("versão inválida".into());
    }
    let dir = install_dir(base, version);
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())
}

/// Versões gerenciadas pelo app (instaladas em `bin/php`).
pub fn managed_versions(base: &Path) -> Vec<String> {
    tools::php_installs(base)
        .into_iter()
        .filter(|i| i.managed)
        .map(|i| i.full_version)
        .collect()
}
