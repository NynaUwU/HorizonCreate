# Como testar no jogo (Horizon)

O mod tem duas metades com o mesmo id (`horizon_create`):

| Pasta | O que é | Sem ela |
|---|---|---|
| `mod/` | **Content mod** (`.hmod`): blocos, itens, receitas, texturas, modelos, idiomas | nada aparece |
| `native/` | **Native mod** (trait mod em Rust): simulação cinética, rotação dos eixos, leitura dos medidores | os blocos existem, mas nada gira e eixos/engrenagens ficam invisíveis |

## 1. Preparar os arquivos

**Atalho:** os arquivos prontos estão em [`releases/v0.2.2/`](../releases/v0.2.2/) (`.hmod` e `.hnmod`).
Para gerar de novo depois de mudar algo:

```sh
python3 tools/gen_textures.py     # só se mudou alguma textura
python3 tools/gen_content.py      # só se mudou blocos/receitas
python3 tools/check_mod.py        # confere o content mod (deve terminar sem ERROR)
python3 tools/sync_native.py      # copia kinetics + hcglue para native/api/src/engine
python3 tools/package.py          # gera dist/horizon_create-<versão>.hmod e .hnmod
```

## 2. Instalar o content mod

Na pasta do jogo (a que tem o `options.txt`):

1. Copie **`horizon_create-0.2.2.hmod`** (ou a pasta `mod/` inteira) para `horizon-mods/`.
2. Abra o jogo → **Opções → Mods** → ligue **Horizon Create**.
3. Como o mod adiciona blocos, o jogo pede para **reiniciar**. Reinicie.
4. Se algo estiver errado, a linha do mod fica **vermelha** com o motivo.

Atalho para desenvolvimento (com a ferramenta do Horizon instalada):

```sh
cargo install --git https://github.com/horizon-rust/horizon cargo-horizon-mod
```

```sh
cd mod && cargo horizon-mod run
```

## 3. Instalar o native mod

1. Copie **`horizon_create-0.2.2.hnmod`** para `horizon-native-mods/` (ou a pasta `native/` para `horizon-native-mods/horizon_create/`).
2. O launcher mostra o aviso de confiança (mods nativos não rodam em sandbox) e **compila o jogo com o mod**.
   A primeira compilação demora bastante.

Ou pela linha de comando:

```sh
horizon-launcher native-build native --instance <sua instância> --trust-native-code
```

> ⚠️ O mod nativo é verificado aqui contra uma **réplica** da API do Horizon
> (`tools/api-check/`, montada a partir do gerador de API da Nightly 6560641), não contra o jogo de verdade.
> A 0.2.1 passa nela sem erros nem avisos, e a réplica reproduz exatamente os erros da 0.1.0. Se ainda
> assim a compilação falhar, mande o `build.log` que o launcher indica.

## Não apareceu nada? (diagnóstico)

1. **Opções → Mods**: devem aparecer duas linhas.
   - **Horizon Create**: o content mod. A chave precisa estar **ligada**, e depois de ligar é preciso **reiniciar** o jogo.
   - **Horizon Create (native)**: o mod nativo (a partir da 0.2.2). Se não aparece, o jogo aberto não é o que o launcher compilou com o mod.
   - Texto **vermelho** na linha = o mod foi recusado; o motivo está escrito ali.
2. **Inventário criativo**: a aba do mod fica **depois das abas vanilla, na página 2** (setas acima do inventário).
   Ou procure `eixo`/`shaft` na aba de busca.
3. **Comando**: `/give @s horizon_create:shaft`. Se disser que o item não existe, o content mod não carregou.
4. **Log**: rode o comando abaixo e procure `horizon_create` ou `content mods not loaded`. Se a montagem do conteúdo
   falha, o jogo abre **sem nenhum content mod** e só avisa no log.

```sh
horizon-launcher logs --lines 400
```

## 4. O que testar (modo criativo, aba **Horizon Create**)

| Teste | Montagem | Resultado esperado |
|---|---|---|
| Motor | Motor Criativo → 3 Eixos → Velocímetro | eixos girando; velocímetro mostra **16 RPM** |
| Velocidade por redstone | bloco de redstone encostado no motor | **256 RPM** (16 × (1 + sinal)) |
| Manivela | Manivela no chão + Medidor de Stress em cima | clicar liga/desliga; **0/256 SU** girando a 32 RPM |
| Engrenagens | Engrenagem Grande + Engrenagem na diagonal, mesmo eixo | a pequena gira ao **dobro** e ao contrário |
| Caixa de engrenagens | eixo entrando por um lado | sai pelos 4 lados; em linha reta **inverte** |
| Embreagem | alavanca encostada | ligada = **corta** a rotação (textura com luz vermelha) |
| Câmbio | alavanca encostada | ligado = **inverte** a saída |
| Corrente | 2 Transmissões por Corrente empilhadas | transmite para o lado |
| Câmbio de corrente | sinal 15 | o que ele aciona gira **2×** |
| Controlador | Engrenagem Grande deitada em cima do controlador | gira em 16 + 16 × sinal RPM |
| Overspeed | controlador a 256 + engrenagem pequena na grande | tudo para (512 RPM > 256) |
| Furadeira | Motor Criativo → eixo → Furadeira (eixo entra por trás) virada para pedra | a broca gira, aparece a rachadura e a pedra quebra (~2,5 s a 32 RPM, 0,5 s a 256 RPM); o medidor mostra o consumo de 4 SU/RPM |
| Furadeira vs. obsidiana | mesma montagem a 16 RPM | demora muito (dureza 50); bedrock nunca quebra |
| Sobrecarga | Manivela (256 SU) → 2 Furadeiras a 32 RPM (2 × 128 SU) | roda; com 3 furadeiras (384 SU) tudo para |

## 5. Limitações desta versão (0.1)

- Só **single player / host de LAN** (mods nativos ainda não rodam em servidor dedicado).
- Velocidade do motor e do controlador é por **redstone** (o Horizon ainda não tem o painel de rolagem do Create).
- Só uma máquina por enquanto: a **Furadeira Mecânica**. Ela ainda não fere entidades na frente
  (o Horizon não tem helper de dano para mods) e usa uma tabela própria de dureza dos blocos vanilla
  (o Horizon também não expõe a dureza); blocos de outros mods contam como dureza 2.
- A parte elétrica (Power Grid) está pronta nas bibliotecas, mas ainda sem blocos no jogo.
