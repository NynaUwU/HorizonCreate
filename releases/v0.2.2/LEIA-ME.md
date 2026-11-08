# Horizon Create 0.2.2 — arquivos prontos

| Arquivo | Onde colocar (pasta do jogo, a que tem `options.txt`) |
|---|---|
| `horizon_create-0.2.2.hmod` | `horizon-mods/` → Opções → Mods → ligar → reiniciar |
| `horizon_create-0.2.2.hnmod` | `horizon-native-mods/` → o launcher compila o jogo com o mod |

- Mods nativos do Horizon são distribuídos **como código-fonte**: o launcher compila junto com o jogo
  na sua máquina (é assim para todos os mods nativos).
- Precisa do Horizon com hooks API **411 ou mais nova** (Nightly de 07/11/2026 em diante).
- `SHA256SUMS` confere os arquivos baixados.

## 0.2.2

- O mod nativo aparece em Opções → Mods como **Horizon Create (native)**, para dar para ver se ele está na build.
- Guia com passo a passo de diagnóstico (`docs/TESTAR-NO-JOGO.md`, "Não apareceu nada?").

## 0.2.1

- Corrige a compilação do mod nativo: `block_entity` e `block_entity_update_tag` são hooks do tipo
  *replace* (devolvem o valor em vez de escrever no evento), `read` ganhou o lifetime explícito e a quebra
  de bloco usa o helper `destroy_block`. Remova versões anteriores de `horizon-native-mods/`.

## 0.2.0

- **Furadeira Mecânica**: consome 4 SU/RPM, quebra o bloco à frente (mais rápido quanto maior o RPM),
  broca girando e rachadura de progresso.
