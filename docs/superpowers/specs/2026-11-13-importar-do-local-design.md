# Importar sites do Local — design

Data: 2026-11-13 · Fork do LocalTusk

## Intenção

Quem usa o Local (localwp.com) deve conseguir trazer um site de lá para o LocalTusk sem
complicação, um site por vez, e terminar com um projeto do LocalTusk funcionando: arquivos,
banco, versões de PHP/MySQL equivalentes e o WordPress já apontando para o novo endereço.
É a porta de entrada para quem está migrando do Local para o LocalTusk.

## Objetivos

- Listar os sites do Local instalados **nesta máquina** e importar o escolhido com um clique.
- Funcionar em qualquer máquina com o Local instalado: macOS e Linux.
- O site importado abre funcionando (WordPress com `wp-config.php` e URLs ajustadas).
- Mensagens claras, em português, quando algo impede ou limita a importação.

## Fora do escopo (agora)

- Importar o `.zip` de "Export" do Local (plano B; o design deve deixá-lo fácil depois).
- Importar de Docker.
- Importar vários sites de uma vez.
- Ler o banco do MySQL do Local enquanto ele roda.
- Traduzir configurações personalizadas do nginx/Apache do Local para o Caddy.
- Windows (o LocalTusk ainda não roda bem lá).

## Anti-objetivos (seria um fracasso mesmo "funcionando")

- **Alterar ou apagar qualquer coisa do Local.** O site lá fica intacto: só lemos e copiamos.
- Deixar um projeto pela metade no LocalTusk quando a importação falha (tem que ser desfeito,
  como já acontece hoje na criação a partir de blueprint).
- Importar um banco desatualizado sem o usuário saber.
- Quebrar a restauração de backup ou a criação por blueprint existentes.

## Como o Local guarda os sites (verificado numa instalação real, Local 10.2, Linux)

- Lista de sites: `sites.json` na pasta de dados do Local:
  - Linux: `~/.config/Local/sites.json`
  - macOS: `~/Library/Application Support/Local/sites.json`
- Cada entrada (chave = id do site, ex.: `lGYzs0rHD`) tem `name`, `path`
  (ex.: `/home/u/Local Sites/snap`; pode vir com `~`), `domain` (ex.: `snap.local`),
  `mysql` (`database: "local"`, `user/password: root`) e `services` com as versões:
  `services.php.version` (ex.: `8.2.29`), `services.mysql.version` (ex.: `8.4.0`) e o servidor
  web (`nginx` ou `apache`). Sites multisite trazem um campo de multisite.
- Estado dos sites: `site-statuses.json` na mesma pasta (`{"<id>": "halted" | "running"}`).
- Pasta do site: `<path>/app/public` (WordPress), `<path>/app/sql/local.sql` (dump do banco),
  `<path>/conf/` (configs do nginx/php/mysql — não usamos).
- O Local grava `app/sql/local.sql` **quando o site é parado**. É um `mysqldump` de um único
  banco, sem `CREATE DATABASE`/`USE`, então entra direto no banco do projeto. Pode ser grande
  (o site de teste tem 500 MB).

## Comportamento

### Onde fica

Na tela de "Novo projeto", uma opção **Importar do Local**, ao lado de "Usar um blueprint".

```
Importar do Local
─────────────────────────────────────────────────────────────
 snap                     PHP 8.2 · MySQL 8.4         [Importar]
 meu-cliente              PHP 7.4 · MySQL 8.0         [Importar]
   Pare o site no Local para importar.                (desativado)
─────────────────────────────────────────────────────────────
 Nome no LocalTusk: [snap        ]
```

- Se o Local não for encontrado (sem `sites.json`), a opção mostra
  "Nenhuma instalação do Local encontrada neste computador."
- Se a lista estiver vazia: "O Local não tem sites."

### Regras por site (decididas com o usuário)

- **Site rodando no Local** (`site-statuses.json` = `running`): botão desativado com
  "Pare o site no Local para importar (se o Local estiver fechado, abra-o e pare o site)."
  Não tentamos ler o banco dele em execução nem detectar processos do Local: o arquivo de
  estado é a única fonte. *(Chamada minha: simples e seguro; o caso "Local fechou travado"
  se resolve abrindo o Local.)*
- **Dump desatualizado** (Local fechou sem parar o site direito): se algum arquivo do banco
  do Local (`<dados do Local>/run/<id>/mysql/data/`) for mais de 2 minutos mais novo que o
  `local.sql`, a importação continua, mas com aviso antes de confirmar: "O banco salvo pelo Local
  pode estar desatualizado. Inicie e pare o site no Local para garantir." A lista mostra a data
  do dump ("banco salvo em 12/11 12:39"). *(Chamada minha. Na instalação real, o Local mexe nesses
  arquivos ~1 s depois de gravar o dump, ao desligar o MySQL — daí a folga.)*
- **Copiar, nunca mover.** Os arquivos são copiados de `app/public`; nada é escrito na pasta do
  Local.
- **Links simbólicos em `app/public`** (plugins/temas linkados de outra pasta): copia-se o
  conteúdo para onde o link aponta, para que o LocalTusk nunca escreva nos arquivos do Local.
  Laços de links são detectados e pulados; as pastas linkadas são listadas no aviso final.
  *(Chamada minha.)*
- **Nome:** o do Local, editável antes de importar.
- **Versões (decidido com o usuário: perguntar na hora):** o PHP e o MySQL vêm do `services`
  do Local. Se a mesma série (`8.2`, `8.4`) estiver instalada no LocalTusk, ela é usada sem
  perguntar. Se não estiver, ao clicar em Importar aparece uma escolha antes de começar:

  ```
  O site usa PHP 8.2, que não está instalado aqui.
  [Instalar o PHP 8.2]   [Usar o PHP 8.3]
  ```

  - "Instalar" baixa a versão pelo mesmo instalador da tela "PHP e ferramentas" (com progresso)
    e depois importa. Se o download falhar, nada é criado e o erro é mostrado.
  - "Usar" oferece a instalada mais próxima (mesma major antes de outra major; a mais nova se
    empatar). *(Chamada minha.)*
  - Se a única alternativa para o MySQL for **mais antiga** que a do Local, o botão "Usar" vem
    com o alerta "O banco pode não importar numa versão mais antiga." *(Chamada minha.)*
  - Se não houver nenhuma versão instalada daquele tipo, só aparece "Instalar".
  - PHP e MySQL faltando ao mesmo tempo: uma única tela com as duas escolhas.
  - Para MySQL, "a mesma série" é a série do Local (`8.4.0` → `8.4`, versão mais nova da série
    no catálogo).
- **Ao terminar:** o projeto novo abre já com `wp-config.php` apontando para o banco do
  LocalTusk e com as URLs trocadas de `http(s)://snap.local` para o endereço do projeto
  (o `wp::retarget` existente faz isso).

### Situações que bloqueiam (com mensagem)

- `app/public` não existe → "A pasta do site não foi encontrada: <caminho>."
- `app/sql/local.sql` não existe → "O Local ainda não salvou o banco deste site. Inicie e pare
  o site no Local e tente de novo."
- O Local usa um banco que não seja MySQL/MariaDB → não suportado (hoje o Local só usa MySQL).

### Situações que só avisam (a importação continua)

- Site multisite → "Sites multisite podem precisar de ajustes manuais."
- Servidor web Apache → "Regras do .htaccess não são usadas pelo LocalTusk."
- Versões substituídas (acima).

## Abordagens consideradas

1. **Converter para um backup `.tar.gz` e chamar a restauração atual.** Quase não mexe no
   código, mas comprime e descompacta centenas de MB à toa (lento e com o dobro de disco).
2. **Ensinar a restauração a aceitar uma pasta como origem (escolhida).** `restore_impl` é
   dividido entre a parte que cria o projeto, sobe o banco e desfaz em caso de erro, e a parte
   que traz o conteúdo (arquivos + dump). O importador entrega uma pasta e um dump direto, sem
   arquivo intermediário. O plano B (.zip) vira "descompactar e usar a mesma função".
3. **Comando de importação independente.** Não toca no código existente (bom para merges do
   fork), mas duplica a parte de desfazer em caso de erro.

**Escolhida: 2**, pelo desempenho em sites reais e por não duplicar a lógica delicada. A mudança
em `restore_impl` deve ser pequena e localizada; backup e blueprint continuam com o mesmo
comportamento.

## Como fica por dentro

- **Novo módulo `src-tauri/src/local_import.rs`** (isolado, para facilitar merges do upstream):
  - acha a pasta de dados do Local por sistema;
  - lê `sites.json` e `site-statuses.json` e devolve a lista de sites (id, nome, caminho,
    domínio, versões de PHP/MySQL, servidor web, multisite, rodando, problemas que bloqueiam);
  - valida um site antes de importar (pastas e dump existem, não está rodando).
- **Restauração com origem genérica:** a parte de conteúdo de `backup::restore_into` passa a
  aceitar, além do `.tar.gz`, uma origem "pasta": copia `<path>/app/public` para a pasta
  pública do projeto e importa `<path>/app/sql/local.sql` com o `import_database` existente;
  depois roda `wp::retarget` como hoje. A criação do projeto (`project_from_manifest` ou
  equivalente) recebe os dados vindos do Local (nome, versões, docroot).
- **Pasta pública:** o conteúdo de `app/public` do Local vai para `app/public` do projeto, com
  `docroot = "public"`, espelhando o Local. *(Chamada minha.)*
- **Novos comandos Tauri:** `list_local_sites` e `import_local_site(local_id, name)`, este com
  progresso pelo mesmo mecanismo de `tracked_install` ("Copiando os arquivos…",
  "Importando o banco de dados…", "Ajustando o WordPress ao novo endereço…") e retornando um
  `RestoreResult` (projeto + aviso).
- **Interface:** a opção em "Novo projeto" e a lista acima, seguindo o padrão visual das telas
  de blueprint; `src/dev-mock.ts` ganha sites de exemplo para testar no navegador.
- **Credenciais do banco:** o Local usa `root/root` e o LocalTusk também, então `retarget` só
  precisa trocar `DB_NAME` e `DB_HOST`, como já faz.

## Testes

- Testes de unidade para a leitura de `sites.json`/`site-statuses.json` (fixtures com um site
  parado, um rodando, um sem `local.sql`, um multisite, caminho com `~`) e para a escolha de
  versões.
- Teste ponta a ponta `#[ignore]` no estilo dos existentes: monta uma pasta falsa do Local
  (WordPress + `local.sql`), importa e confere que o site responde com o novo endereço.
- Os testes de backup e blueprint existentes continuam passando.

## Deixado para quem for construir

- Nomes exatos de tipos e funções, e como dividir `restore_impl` sem mudar o comportamento atual.
- Como copiar os arquivos de forma eficiente e com progresso.
- Layout exato da tela, desde que siga o padrão das telas existentes.
