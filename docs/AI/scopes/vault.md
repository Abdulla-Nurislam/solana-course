# Область `vault` — Программа `liquidity-vault` и клиент `vault-client`

## 1. Структура отдельного Workspace

Проект `liquidity-vault` выделен в самостоятельный Cargo/Anchor workspace:
- `programs/liquidity-vault` — ончейн-программа на Anchor `1.2.0` (`anchor-spl = 1.2.2`).
- `clients/vault-client` — CLI-клиент на Rust без внешних TypeScript SDK для управления хранилищем, пулом Raydium CLMM, стратегией, обменами и концентрированной позицией.

## 2. Сборка SBF и локальный валидатор / тесты

1. Сборка SBF-бинарника программы:
   ```bash
   cargo build-sbf --manifest-path programs/liquidity-vault/Cargo.toml
   ```
   Артефакт создаётся в `target/deploy/liquidity_vault.so`.
2. Запуск интеграционных тестов LiteSVM:
   ```bash
   cargo test -p liquidity-vault --test vault
   ```
3. Деплой в локальный валидатор (`solana-test-validator`) или в `devnet`:
   ```bash
   solana program deploy target/deploy/liquidity_vault.so \
     --program-id target/deploy/liquidity_vault-keypair.json \
     --url devnet
   ```

## 3. Архитектура PDA и владение средствами

- `VaultState` (`seeds = [b"vault", deposit_mint.as_ref()]`) — центральный PDA хранилища, который выступает:
  - `mint::authority` для `share_mint` (`seeds = [b"share_mint", vault.key().as_ref()]`);
  - `token::authority` для основного хранилища `token_vault` (`seeds = [b"vault_token", vault.key().as_ref()]`);
  - `token::authority` для второго хранилища `secondary_vault` (`seeds = [b"strategy_vault", vault.key().as_ref()]`);
  - владельцем NFT-позиции Raydium CLMM (`position_nft_account`).
- `UserCard` (`seeds = [b"user_card", vault.key().as_ref(), user.key().as_ref()]`) — персональная карточка вкладчика.
- `Strategy` (`seeds = [b"strategy", vault.key().as_ref()]`) — привязка vault к пулу Raydium CLMM, второму токену и активной CLMM-позиции.

## 4. Ручной CPI в Raydium CLMM и источник котировки

- Вместо зависимости от устаревших крейтов `raydium-contract-instructions` с конфликтующими версиями `solana-program`/`anchor-lang`, модуль `programs/liquidity-vault/src/raydium.rs` формирует инструкции Raydium CLMM (`swap_v2`, `open_position_with_token22_nft`, `increase_liquidity_v2`, `decrease_liquidity_v2`, `close_position`) напрямую по 8-байтным дискриминаторам Anchor и проверяет бинарную раскладку `PoolState` и `PersonalPositionState`.
- Перед выполнением `vault-swap`, `open-vault-position` и `close-vault-position` клиент `vault-client` выполняет `simulateTransaction` с чтением целевых токен-аккаунтов после симуляции. Из дельты балансов вычисляется точная ожидаемая сумма (`expected_out`) и задаётся порог защиты от проскальзывания (`min_amount_out = expected_out * (10_000 - slippage_bps) / 10_000`).
- Для демонстрации отклонения по slippage (`--reject`) транзакция отправляется с `skipPreflight: true` и завышенным `min_amount_out`, чтобы отказ `SlippageExceeded` зафиксировался в блокчейне и был виден в Solana Explorer.

