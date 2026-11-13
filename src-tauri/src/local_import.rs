//! Leitura dos sites do Local (localwp.com): lê `sites.json` e
//! `site-statuses.json` e diz o estado de cada site, sem mexer em nada.

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MSG_RODANDO: &str = "Pare o site no Local antes de importar.";
const MSG_SEM_PASTA: &str = "A pasta do site não foi encontrada.";
const MSG_SEM_DUMP: &str = "O Local ainda não salvou o banco deste site. Abra o site no Local e pare-o para o banco ser salvo.";
const MSG_MULTISITE: &str = "Site multisite: os endereços do banco podem precisar de ajuste depois da importação.";
const MSG_APACHE: &str = "O site usa Apache no Local; aqui ele roda com o servidor padrão do LocalTusk.";
const MSG_DUMP_VELHO: &str = "O banco salvo está mais antigo que o banco em uso no Local; pare o site no Local para atualizá-lo.";

/// Folga entre o dump e o banco vivo antes de considerar o dump desatualizado.
const FOLGA_DUMP: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSite {
    pub id: String,
    pub name: String,
    pub path: String,
    pub domain: String,
    /// Versão completa do Local, ex. "8.2.29" ("" se ausente).
    pub php_version: String,
    /// Ex. "8.4.0".
    pub mysql_version: String,
    /// "nginx" | "apache" | "".
    pub web_server: String,
    pub multisite: bool,
    pub running: bool,
    /// mtime do local.sql em segundos desde 1970; 0 se não existe.
    pub dump_saved_at: u64,
    pub dump_stale: bool,
    /// Mensagem que impede a importação.
    pub blocker: Option<String>,
    /// Avisos: multisite, apache, dump desatualizado.
    pub warnings: Vec<String>,
}

/// Pasta de dados do Local neste sistema (`None` no Windows, sem suporte).
pub fn local_data_dir(home: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        Some(home.join(".config/Local"))
    } else if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Local"))
    } else {
        None
    }
}

pub fn public_dir(site: &LocalSite) -> PathBuf {
    Path::new(&site.path).join("app/public")
}

pub fn dump_file(site: &LocalSite) -> PathBuf {
    Path::new(&site.path).join("app/sql/local.sql")
}

/// "8.2.29" -> "8.2".
pub fn series(full: &str) -> String {
    full.split('.').take(2).collect::<Vec<_>>().join(".")
}

pub fn list_sites(data_dir: &Path, home: &Path) -> Result<Vec<LocalSite>, String> {
    let file = data_dir.join("sites.json");
    let raw = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let sites: Value = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", file.display()))?;
    let obj = sites.as_object().ok_or_else(|| format!("{}: formato inesperado", file.display()))?;
    let statuses: Value = fs::read_to_string(data_dir.join("site-statuses.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);

    let mut out: Vec<LocalSite> = obj
        .iter()
        .map(|(id, v)| build_site(data_dir, home, id, v, statuses.get(id).and_then(Value::as_str)))
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

pub fn find_site(data_dir: &Path, home: &Path, id: &str) -> Result<LocalSite, String> {
    list_sites(data_dir, home)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("Site '{id}' não encontrado no Local."))
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Maior mtime de qualquer arquivo dentro de `dir`, recursivamente.
fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut best: Option<SystemTime> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        let t = if p.is_dir() { newest_mtime(&p) } else { mtime(&p) };
        if let Some(t) = t {
            if best.map_or(true, |b| t > b) {
                best = Some(t);
            }
        }
    }
    best
}

fn build_site(data_dir: &Path, home: &Path, id: &str, v: &Value, status: Option<&str>) -> LocalSite {
    let raw_path = text(v, "path");
    let path = match raw_path.strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches('/')).display().to_string(),
        None => raw_path,
    };
    let services = v.get("services");
    let service_version = |name: &str| {
        services
            .and_then(|s| s.get(name))
            .and_then(|s| s.get("version"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let has_service = |name: &str| services.and_then(|s| s.get(name)).is_some();
    let web_server = if has_service("apache") {
        "apache"
    } else if has_service("nginx") {
        "nginx"
    } else {
        ""
    };
    let multisite = v
        .get("multiSite")
        .and_then(Value::as_str)
        .is_some_and(|m| !m.is_empty() && m != "no");

    let mut site = LocalSite {
        id: id.to_string(),
        name: text(v, "name"),
        path,
        domain: text(v, "domain"),
        php_version: service_version("php"),
        mysql_version: service_version("mysql"),
        web_server: web_server.to_string(),
        multisite,
        running: status == Some("running"),
        dump_saved_at: 0,
        dump_stale: false,
        blocker: None,
        warnings: Vec::new(),
    };

    let dump_time = mtime(&dump_file(&site));
    if let Some(t) = dump_time {
        site.dump_saved_at = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let live = newest_mtime(&data_dir.join("run").join(id).join("mysql/data"));
        site.dump_stale = live.is_some_and(|l| l > t + FOLGA_DUMP);
    }

    site.blocker = if site.running {
        Some(MSG_RODANDO.to_string())
    } else if site.path.is_empty() || !Path::new(&site.path).is_dir() {
        Some(MSG_SEM_PASTA.to_string())
    } else if dump_time.is_none() {
        Some(MSG_SEM_DUMP.to_string())
    } else {
        None
    };

    if site.multisite {
        site.warnings.push(MSG_MULTISITE.to_string());
    }
    if site.web_server == "apache" {
        site.warnings.push(MSG_APACHE.to_string());
    }
    if site.dump_stale {
        site.warnings.push(MSG_DUMP_VELHO.to_string());
    }
    site
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_base(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("localtusk-li-{tag}-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(p: &Path, content: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    /// Monta um "Local" falso e devolve (data_dir, home).
    fn fake_local(dir: &Path) -> (PathBuf, PathBuf) {
        let home = dir.join("home");
        let data = home.join(".config/Local");
        fs::create_dir_all(&data).unwrap();
        let sites = json!({
            "snap1": {"name": "snap", "path": "~/Local Sites/snap", "domain": "snap.local",
                "services": {"php": {"version": "8.2.29"}, "nginx": {"version": "1.26.1"}, "mysql": {"version": "8.4.0"}}},
            "run1": {"name": "rodando", "path": "~/Local Sites/rodando", "domain": "rodando.local",
                "services": {"php": {"version": "8.1.0"}, "nginx": {}, "mysql": {"version": "8.0.0"}}},
            "nodump": {"name": "semdump", "path": "~/Local Sites/semdump", "services": {"nginx": {}}},
            "gone": {"name": "sumiu", "path": "~/Local Sites/sumiu", "services": {}},
            "bare": {"name": "nu", "path": "~/Local Sites/nu"},
            "ms": {"name": "multi", "path": "~/Local Sites/multi", "multiSite": "ms-subdir",
                "services": {"apache": {}, "php": {"version": "8.2.0"}}},
        });
        write(&data.join("sites.json"), &sites.to_string());
        write(
            &data.join("site-statuses.json"),
            &json!({"snap1": "halted", "run1": "running", "nodump": "halted", "ms": "halted"}).to_string(),
        );
        for (name, dump) in [("snap", true), ("rodando", true), ("semdump", false), ("nu", true), ("multi", true)] {
            let root = home.join("Local Sites").join(name);
            fs::create_dir_all(root.join("app/public")).unwrap();
            if dump {
                write(&root.join("app/sql/local.sql"), "-- dump");
            }
        }
        (data, home)
    }

    fn get(sites: &[LocalSite], id: &str) -> LocalSite {
        sites.iter().find(|x| x.id == id).unwrap().clone()
    }

    #[test]
    fn lista_site_parado_pronto() {
        let dir = temp_base("pronto");
        let (data, home) = fake_local(&dir);
        let s = list_sites(&data, &home).unwrap();
        let snap = s.iter().find(|x| x.id == "snap1").unwrap();
        assert_eq!(snap.path, home.join("Local Sites/snap").display().to_string());
        assert_eq!((snap.php_version.as_str(), snap.mysql_version.as_str(), snap.web_server.as_str()), ("8.2.29", "8.4.0", "nginx"));
        assert!(snap.blocker.is_none() && !snap.running && snap.dump_saved_at > 0);
        assert!(snap.warnings.is_empty());
        let names: Vec<_> = s.iter().map(|x| x.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "ordenado por nome");
    }

    #[test]
    fn site_rodando_fica_bloqueado() {
        let dir = temp_base("rodando");
        let (data, home) = fake_local(&dir);
        let s = get(&list_sites(&data, &home).unwrap(), "run1");
        assert!(s.running);
        assert_eq!(s.blocker.as_deref(), Some(MSG_RODANDO));
        assert!(s.blocker.unwrap().contains("Pare o site no Local"));
    }

    #[test]
    fn site_sem_dump_fica_bloqueado() {
        let dir = temp_base("semdump");
        let (data, home) = fake_local(&dir);
        let s = get(&list_sites(&data, &home).unwrap(), "nodump");
        assert!(s.blocker.unwrap().contains("ainda não salvou o banco"));
        assert_eq!(s.dump_saved_at, 0);
    }

    #[test]
    fn site_sem_pasta_fica_bloqueado_e_lista_continua() {
        let dir = temp_base("sempasta");
        let (data, home) = fake_local(&dir);
        let all = list_sites(&data, &home).unwrap();
        let s = get(&all, "gone");
        assert!(s.blocker.unwrap().contains("pasta do site não foi encontrada"));
        assert!(get(&all, "snap1").blocker.is_none());
    }

    #[test]
    fn campos_faltando_nao_derrubam() {
        let dir = temp_base("faltando");
        let (data, home) = fake_local(&dir);
        let s = get(&list_sites(&data, &home).unwrap(), "bare");
        assert_eq!(s.php_version, "");
        assert_eq!(s.mysql_version, "");
        assert_eq!(s.web_server, "");
        assert_eq!(s.domain, "");
    }

    #[test]
    fn multisite_e_apache_avisam() {
        let dir = temp_base("multi");
        let (data, home) = fake_local(&dir);
        let s = get(&list_sites(&data, &home).unwrap(), "ms");
        assert!(s.multisite);
        assert_eq!(s.web_server, "apache");
        assert!(s.warnings.contains(&MSG_MULTISITE.to_string()));
        assert!(s.warnings.contains(&MSG_APACHE.to_string()));
        assert!(s.blocker.is_none());
    }

    #[test]
    fn dump_desatualizado_avisa() {
        let dir = temp_base("stale");
        let (data, home) = fake_local(&dir);
        let dump = home.join("Local Sites/snap/app/sql/local.sql");
        let dump_time = mtime(&dump).unwrap();
        let ibdata = data.join("run/snap1/mysql/data/ibdata1");
        write(&ibdata, "x");
        let set = |t: SystemTime| fs::File::options().write(true).open(&ibdata).unwrap().set_modified(t).unwrap();

        set(dump_time + Duration::from_secs(300));
        let s = get(&list_sites(&data, &home).unwrap(), "snap1");
        assert!(s.dump_stale);
        assert!(s.warnings.contains(&MSG_DUMP_VELHO.to_string()));
        assert!(s.blocker.is_none());

        set(dump_time + Duration::from_secs(1));
        let s = get(&list_sites(&data, &home).unwrap(), "snap1");
        assert!(!s.dump_stale);
        assert!(s.warnings.is_empty());
    }

    #[test]
    fn sem_local_instalado_da_erro() {
        let dir = temp_base("semlocal");
        assert!(list_sites(&dir, &dir).is_err());
        assert!(find_site(&dir, &dir, "x").is_err());
    }

    #[test]
    fn serie_da_versao() {
        assert_eq!(series("8.2.29"), "8.2");
        assert_eq!(series("8.4"), "8.4");
        assert_eq!(series(""), "");
    }

    #[test]
    fn acha_site_por_id() {
        let dir = temp_base("find");
        let (data, home) = fake_local(&dir);
        assert_eq!(find_site(&data, &home, "snap1").unwrap().name, "snap");
        assert!(find_site(&data, &home, "nope").is_err());
        assert!(public_dir(&find_site(&data, &home, "snap1").unwrap()).ends_with("snap/app/public"));
        assert!(dump_file(&find_site(&data, &home, "snap1").unwrap()).ends_with("app/sql/local.sql"));
        assert!(local_data_dir(&home).is_some() || cfg!(windows));
    }
}
