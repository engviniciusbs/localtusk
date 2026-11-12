//! Blueprints: modelos de projeto reutilizáveis.
//!
//! Um blueprint é um projeto "congelado": os arquivos, o banco de dados e as versões de PHP e
//! MySQL de um projeto existente. Fica em `~/LocalTusk/blueprints/` e usa o mesmo formato do
//! backup (`.tar.gz` com `manifest.json`, `database.sql` e `app/`), com `kind = "blueprint"`
//! no manifesto. Criar um projeto a partir dele reaproveita a restauração de backup.

use crate::backup::{self, BlueprintMeta, EXT, Manifest};
use crate::download::InstallProgress;
use crate::project::{self, Project};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub const KIND: &str = "blueprint";
const MAX_NAME: usize = 60;
const MAX_DESCRIPTION: usize = 280;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintInfo {
    pub file_name: String,
    pub path: String,
    pub name: String,
    pub description: String,
    pub size: u64,
    pub created_at: u64,
    /// Nome do projeto a partir do qual o blueprint foi criado.
    pub source_name: String,
    pub php_version: String,
    pub db_version: String,
    pub docroot: String,
    pub has_wordpress: bool,
}

pub fn blueprints_dir(base: &Path) -> PathBuf {
    base.join("blueprints")
}

/// Valida o nome dado ao blueprint (sem quebras de linha, não vazio, tamanho limitado).
pub fn clean_name(raw: &str) -> Result<String, String> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Dê um nome ao blueprint.".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("O nome do blueprint pode ter no máximo {MAX_NAME} caracteres."));
    }
    Ok(name)
}

pub fn clean_description(raw: &str) -> String {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    text.chars().take(MAX_DESCRIPTION).collect()
}

/// Lê o manifesto e garante que o arquivo é um blueprint (e não um backup comum).
pub fn read(path: &Path) -> Result<Manifest, String> {
    let m = backup::read_manifest(path)?;
    if m.kind != KIND {
        return Err(
            "Este arquivo é um backup, não um blueprint. Para usá-lo, escolha \"Restaurar backup\".".into(),
        );
    }
    Ok(m)
}

pub fn info(path: &Path) -> Result<BlueprintInfo, String> {
    let m = read(path)?;
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    Ok(BlueprintInfo {
        file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: path.display().to_string(),
        name: if m.blueprint_name.is_empty() { m.name.clone() } else { m.blueprint_name },
        description: m.description,
        size: meta.len(),
        created_at: m.created_at,
        source_name: m.name,
        php_version: m.php_version,
        db_version: m.db_version,
        docroot: m.docroot,
        has_wordpress: m.has_wordpress,
    })
}

/// Blueprints guardados em `~/LocalTusk/blueprints`, do mais novo para o mais antigo.
pub fn list(base: &Path) -> Vec<BlueprintInfo> {
    let Ok(rd) = fs::read_dir(blueprints_dir(base)) else {
        return vec![];
    };
    let mut all: Vec<BlueprintInfo> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(EXT))
        .filter_map(|p| info(&p).ok())
        .collect();
    all.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.file_name.cmp(&b.file_name)));
    all
}

/// Caminho livre para um blueprint chamado `name`: `<slug>.tar.gz`, `<slug>-2.tar.gz`…
fn unique_target(base: &Path, name: &str) -> PathBuf {
    let dir = blueprints_dir(base);
    let slug = project::slugify(name);
    let mut target = dir.join(format!("{slug}{EXT}"));
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{slug}-{n}{EXT}"));
        n += 1;
    }
    target
}

/// Cria um blueprint a partir do projeto `p` (o banco precisa estar rodando).
pub fn create(
    base: &Path,
    p: &Project,
    name: &str,
    description: &str,
    key: &str,
    progress: &dyn Fn(InstallProgress),
) -> Result<PathBuf, String> {
    let dir = blueprints_dir(base);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let target = unique_target(base, name);
    backup::write_archive(base, p, &target, Some(BlueprintMeta { name, description }), key, progress)
}

/// Resolve `path` e confere que é um blueprint dentro da pasta de blueprints.
fn inside_dir(base: &Path, path: &Path) -> Result<PathBuf, String> {
    let dir = blueprints_dir(base)
        .canonicalize()
        .map_err(|_| "Pasta de blueprints não encontrada.".to_string())?;
    let file = path.canonicalize().map_err(|_| "Blueprint não encontrado.".to_string())?;
    if !file.starts_with(&dir) || !file.to_string_lossy().ends_with(EXT) {
        return Err("Este arquivo não está na pasta de blueprints.".into());
    }
    Ok(file)
}

/// Apaga um blueprint. Só aceita arquivos dentro da pasta de blueprints.
pub fn delete(base: &Path, path: &Path) -> Result<(), String> {
    let file = inside_dir(base, path)?;
    fs::remove_file(file).map_err(|e| e.to_string())
}

/// Copia um arquivo de blueprint de fora para a pasta de blueprints.
pub fn import(base: &Path, src: &Path) -> Result<BlueprintInfo, String> {
    let m = read(src)?;
    let name = if m.blueprint_name.is_empty() { &m.name } else { &m.blueprint_name };
    let dir = blueprints_dir(base);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let target = unique_target(base, name);
    let part = PathBuf::from(format!("{}.part", target.display()));
    if let Err(e) = fs::copy(src, &part) {
        let _ = fs::remove_file(&part);
        return Err(format!("Não foi possível importar o blueprint: {e}"));
    }
    fs::rename(&part, &target).map_err(|e| e.to_string())?;
    info(&target)
}

/// Copia um blueprint da pasta de blueprints para `dest` (para compartilhar com outras pessoas).
pub fn export(base: &Path, path: &Path, dest: &Path) -> Result<PathBuf, String> {
    let file = inside_dir(base, path)?;
    let mut dest = dest.to_path_buf();
    if !dest.to_string_lossy().ends_with(EXT) {
        dest = PathBuf::from(format!("{}{EXT}", dest.display()));
    }
    if dest.canonicalize().map(|d| d == file).unwrap_or(false) {
        return Err("Escolha um destino diferente do arquivo original.".into());
    }
    fs::copy(&file, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    fn temp_base(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("localtusk-bp-{tag}-{}-{:?}", project::now(), std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Monta um arquivo no formato do backup, sem precisar de MySQL.
    fn fake_archive(path: &Path, kind: &str, blueprint_name: &str) {
        let m = Manifest {
            format: 1,
            id: "loja".into(),
            name: "Loja".into(),
            docroot: "public".into(),
            php_version: "8.3".into(),
            db_version: "8.4".into(),
            db_name: "loja".into(),
            http_port: 8001,
            db_port: 3307,
            site_url: "http://loja.localhost:8001".into(),
            has_wordpress: false,
            created_at: 1_700_000_000,
            kind: kind.into(),
            blueprint_name: blueprint_name.into(),
            description: "Base com login pronto".into(),
        };
        let json = serde_json::to_vec_pretty(&m).unwrap();
        let gz = GzEncoder::new(fs::File::create(path).unwrap(), Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut h = tar::Header::new_gnu();
        h.set_size(json.len() as u64);
        h.set_mode(0o644);
        tar.append_data(&mut h, backup::MANIFEST, json.as_slice()).unwrap();
        let sql = b"-- dump";
        let mut h = tar::Header::new_gnu();
        h.set_size(sql.len() as u64);
        h.set_mode(0o644);
        tar.append_data(&mut h, backup::DUMP, &sql[..]).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn valida_nome_e_descricao() {
        assert_eq!(clean_name("  Landing   page \n").unwrap(), "Landing page");
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"a".repeat(MAX_NAME + 1)).is_err());
        assert_eq!(clean_name(&"é".repeat(MAX_NAME)).unwrap().chars().count(), MAX_NAME);
        assert_eq!(clean_description("  uma\n\ndescrição  "), "uma descrição");
        assert_eq!(clean_description(&"x".repeat(1000)).chars().count(), MAX_DESCRIPTION);
    }

    #[test]
    fn importa_lista_exporta_e_apaga() {
        let base = temp_base("fluxo");
        let src = base.join("recebido.tar.gz");
        fake_archive(&src, KIND, "Landing page");

        let imported = import(&base, &src).unwrap();
        assert_eq!(imported.name, "Landing page");
        assert_eq!(imported.source_name, "Loja");
        assert_eq!(imported.description, "Base com login pronto");
        assert_eq!(imported.php_version, "8.3");
        assert!(imported.path.ends_with("blueprints/landing-page.tar.gz"), "{}", imported.path);

        // Importar de novo não sobrescreve: ganha um sufixo.
        let again = import(&base, &src).unwrap();
        assert!(again.path.ends_with("blueprints/landing-page-2.tar.gz"), "{}", again.path);
        assert_eq!(list(&base).len(), 2);

        // Exportar acrescenta a extensão quando falta.
        let out = base.join("para-enviar");
        let exported = export(&base, Path::new(&imported.path), &out).unwrap();
        assert!(exported.to_string_lossy().ends_with(".tar.gz"));
        assert_eq!(read(&exported).unwrap().blueprint_name, "Landing page");
        // Exportar sobre o próprio arquivo é recusado.
        assert!(export(&base, Path::new(&imported.path), Path::new(&imported.path)).is_err());

        delete(&base, Path::new(&imported.path)).unwrap();
        assert_eq!(list(&base).len(), 1);
        // Só apaga o que está na pasta de blueprints.
        assert!(delete(&base, &src).is_err());
        assert!(src.exists());
        assert!(export(&base, &exported, &base.join("x.tar.gz")).is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn recusa_backup_comum_e_arquivo_invalido() {
        let base = temp_base("recusa");
        let backup_file = base.join("backup.tar.gz");
        fake_archive(&backup_file, "", "");
        let err = import(&base, &backup_file).unwrap_err();
        assert!(err.contains("Restaurar backup"), "{err}");

        let lixo = base.join("lixo.tar.gz");
        fs::write(&lixo, b"nao sou um tar").unwrap();
        assert!(import(&base, &lixo).is_err());
        assert!(list(&base).is_empty());

        // Um backup colocado na pasta de blueprints não aparece na lista.
        fs::create_dir_all(blueprints_dir(&base)).unwrap();
        fake_archive(&blueprints_dir(&base).join("intruso.tar.gz"), "", "");
        assert!(list(&base).is_empty());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn sem_nome_do_blueprint_usa_o_do_projeto() {
        let base = temp_base("semnome");
        let src = base.join("a.tar.gz");
        fake_archive(&src, KIND, "");
        assert_eq!(import(&base, &src).unwrap().name, "Loja");
        let _ = fs::remove_dir_all(&base);
    }
}
