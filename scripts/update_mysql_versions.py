#!/usr/bin/env python3
"""Gera src-tauri/resources/mysql-versions.json, o catálogo de versões do MySQL embutido no app.

O app NÃO sonda servidores em tempo de execução (isso é lento e o CDN da Oracle bloqueia
rajadas de requisições). Quando sair uma versão nova do MySQL, rode este script e faça commit
do JSON atualizado.

Uso:  python3 scripts/update_mysql_versions.py

É educado de propósito: uma requisição por vez, com pausa entre elas, e aborta se o servidor
começar a recusar (403/429).
"""
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import date
from pathlib import Path

# série -> maior patch a sondar
SERIES = {"8.0": 50, "8.4": 20, "9.7": 10}
# rótulos de sistema, do mais novo ao mais antigo
LABELS = ["macos15", "macos14", "macos13", "macos12"]
ARCHS = ["arm64", "x86_64"]
CDN = "https://cdn.mysql.com"
PAUSE = 0.12
OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "resources" / "mysql-versions.json"

requests = 0


def head(url):
    """Tamanho em bytes se o arquivo existe, None se 404."""
    global requests
    requests += 1
    time.sleep(PAUSE)
    req = urllib.request.Request(url, method="HEAD", headers={"User-Agent": "LocalTusk-catalog/0.1"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return int(r.headers.get("Content-Length", 0))
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        sys.exit(f"Servidor recusou ({e.code}) em {url}. Abortando para não insistir; tente mais tarde.")


def candidates(series):
    # Versão atual da série fica em Downloads/; as anteriores em archives/.
    return [f"Downloads/MySQL-{series}", f"archives/mysql-{series}"]


def find_build(series, version, arch, hint=None):
    """Procura (caminho, rótulo) que existe para esta versão/arquitetura."""
    combos = [(p, l) for p in candidates(series) for l in LABELS]
    if hint:
        combos.remove(hint) if hint in combos else None
        combos.insert(0, hint)
    for path, label in combos:
        size = head(f"{CDN}/{path}/mysql-{version}-{label}-{arch}.tar.gz")
        if size is not None:
            return {"path": path, "label": label, "bytes": size}
    return None


def main():
    versions = []
    for series, max_patch in SERIES.items():
        missing_streak = 0
        for patch in range(max_patch + 1):
            version = f"{series}.{patch}"
            arm = find_build(series, version, "arm64")
            if not arm:
                missing_streak += 1
                print(f"  {version}: -")
                # Séries têm lacunas, mas muitas ausências seguidas depois de achar algo = fim.
                if missing_streak >= 6 and any(v["series"] == series for v in versions):
                    break
                continue
            missing_streak = 0
            intel = find_build(series, version, "x86_64", hint=(arm["path"], arm["label"]))
            builds = {"arm64": arm}
            if intel:
                builds["x86_64"] = intel
            versions.append({"version": version, "series": series, "builds": builds})
            print(f"  {version}: {', '.join(builds)}  ({arm['label']}, {arm['bytes'] // 1024 // 1024} MB)")

    versions.sort(key=lambda v: tuple(int(x) for x in v["version"].split(".")), reverse=True)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"generated": date.today().isoformat(), "versions": versions}, indent=1) + "\n")
    print(f"\n{len(versions)} versões gravadas em {OUT} ({requests} requisições)")


if __name__ == "__main__":
    main()
