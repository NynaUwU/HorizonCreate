# Como testar no jogo (Horizon)

O mod tem duas metades com o mesmo id (`horizon_create`):

| Pasta | O que é | Sem ela |
|---|---|---|
| `mod/` | **Content mod** (`.hmod`): blocos, itens, receitas, texturas, modelos, idiomas | nada aparece |
| `native/` | **Native mod** (trait mod em Rust): simulação cinética, rotação dos eixos, leitura dos medidores | os blocos existem, mas nada gira e eixos/engrenagens ficam invisíveis |

## 1. Preparar os arquivos

```sh
python3 tools/gen_textures.py     # só se mudou alguma textura
python3 tools/gen_content.py      # só se mudou blocos/receitas
python3 tools/check_mod.py        # confere o content mod (deve terminar sem ERROR)
python3 tools/sync_native.py      # copia kinetics + hcglue para native/api/src/engine
python3 tools/package.py          # gera dist/horizon_create.hmod
```

## 2. Instalar o content mod

Na pasta do jogo (a que tem o `options.txt`):

1. Copie **`dist/horizon_create.hmod`** (ou a pasta `mod/` inteira) para `horizon-mods/`.
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

1. Copie a pasta **`native/`** para `horizon-native-mods/horizon_create/` (na pasta do jogo).
2. O launcher mostra o aviso de confiança (mods nativos não rodam em sandbox) e **compila o jogo com o mod**.
   A primeira compilação demora bastante.

Ou pela linha de comando:

```sh
horizon-launcher native-build native --instance <sua instância> --trust-native-code
```

> ⚠️ O adaptador `native/api/src/lib.rs` foi escrito seguindo a API do Horizon, mas **não pude compilá-lo
> aqui** (o ambiente não deixa compilar o código do Horizon). A lógica em si (`engine/`) é testada e compila.
> Se a compilação falhar, me mande o `build.log` que o launcher indica: em geral são ajustes pequenos de nome
> ou tipo.

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

## 5. Limitações desta versão (0.1)

- Só **single player / host de LAN** (mods nativos ainda não rodam em servidor dedicado).
- Velocidade do motor e do controlador é por **redstone** (o Horizon ainda não tem o painel de rolagem do Create).
- Sem máquinas consumidoras ainda (prensa, misturador...): o medidor de stress mostra só a capacidade.
- A parte elétrica (Power Grid) está pronta nas bibliotecas, mas ainda sem blocos no jogo.
