# Horizon Create 0.1.0 — arquivos prontos

| Arquivo | Onde colocar (pasta do jogo, a que tem `options.txt`) |
|---|---|
| `horizon_create-0.1.0.hmod` | `horizon-mods/` → Opções → Mods → ligar → reiniciar |
| `horizon_create-0.1.0.hnmod` | `horizon-native-mods/` → o launcher compila o jogo com o mod |

- Mods nativos do Horizon são distribuídos **como código-fonte**: o launcher compila junto com o jogo
  na sua máquina (é assim para todos os mods nativos, não dá para enviar um binário pronto).
- Precisa do Horizon com hooks API **411 ou mais nova** (Nightly de 07/11/2026 em diante).
- Testado contra a Nightly `6560641`. O adaptador nativo ainda não foi compilado contra o jogo:
  se a compilação falhar, mande o `build.log` que o launcher indica.
- O que testar: `docs/TESTAR-NO-JOGO.md`.
