# Область `vault` — Программа `liquidity-vault` и клиент `vault-client`

## 1. Структура отдельного Workspace

Проект `liquidity-vault` выделен в самостоятельный Cargo/Anchor workspace:
- `programs/liquidity-vault` — ончейн-программа на Anchor `1.2.0` (`anchor-spl = 1.2.2`).
- `clients/vault-client` — CLI-клиент на Rust для управления хранилищем.

## 2. Сборка SBF и тестирование

1. Сборка SBF-бинарника программы:
   ```bash
   cargo build-sbf --manifest-path programs/liquidity-vault/Cargo.toml
   ```
   Артефакт создаётся в `target/deploy/liquidity_vault.so`.
2. Запуск интеграционных тестов LiteSVM:
   ```bash
   cargo test -p liquidity-vault --test vault
   ```
