use std::str::FromStr;

use anchor_lang::{
    error::ERROR_CODE_OFFSET,
    prelude::Pubkey,
    solana_program::instruction::Instruction,
    system_program, AccountDeserialize, InstructionData, ToAccountMetas,
};
use litesvm::LiteSVM;
use liquidity_vault::{
    accounts,
    error::VaultError,
    instruction,
    raydium::POOL_STATE_DISCRIMINATOR,
    state::{
        Strategy, UserCard, VaultState, SHARE_MINT_SEED, STRATEGY_SEED, STRATEGY_VAULT_SEED,
        USER_CARD_SEED, VAULT_SEED, VAULT_TOKEN_SEED,
    },
};
use solana_account::Account;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

const DECIMALS: u8 = 6;
const ONE_TOKEN: u64 = 1_000_000;
const SOL: u64 = 1_000_000_000;

fn pk(s: &str) -> Pubkey {
    Pubkey::from_str(s).unwrap()
}

fn token_program() -> Pubkey {
    pk("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
}

fn token_2022_program() -> Pubkey {
    pk("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")
}

fn ata_program() -> Pubkey {
    pk("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")
}

fn memo_program() -> Pubkey {
    pk("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")
}

fn rent_sysvar() -> Pubkey {
    pk("SysvarRent111111111111111111111111111111111")
}

fn mock_clmm_program() -> Pubkey {
    pk("devi51mZmdwUJGU9hjN27vEz64Gps7uUefqxg27EAtH")
}

fn custom_err(e: VaultError) -> String {
    format!("Custom({})", ERROR_CODE_OFFSET + e as u32)
}

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &liquidity_vault::id()).0
}

fn ata(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program().as_ref(), mint.as_ref()],
        &ata_program(),
    )
    .0
}

fn mint_bytes(decimals: u8, supply: u64, mint_authority: &Pubkey) -> Vec<u8> {
    let mut d = vec![0u8; 82];
    d[0..4].copy_from_slice(&1u32.to_le_bytes()); // COption::Some
    d[4..36].copy_from_slice(mint_authority.as_ref());
    d[36..44].copy_from_slice(&supply.to_le_bytes());
    d[44] = decimals;
    d[45] = 1; // is_initialized = true
    d
}

fn token_account_bytes(mint: &Pubkey, owner: &Pubkey, amount: u64) -> Vec<u8> {
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(mint.as_ref());
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1; // AccountState::Initialized
    d
}

fn mock_pool_state_bytes(
    amm_config: &Pubkey,
    mint_0: &Pubkey,
    mint_1: &Pubkey,
    vault_0: &Pubkey,
    vault_1: &Pubkey,
    observation: &Pubkey,
) -> Vec<u8> {
    let mut d = vec![0u8; 1544];
    d[0..8].copy_from_slice(&POOL_STATE_DISCRIMINATOR);
    d[8] = 254;
    d[9..41].copy_from_slice(amm_config.as_ref());
    d[73..105].copy_from_slice(mint_0.as_ref());
    d[105..137].copy_from_slice(mint_1.as_ref());
    d[137..169].copy_from_slice(vault_0.as_ref());
    d[169..201].copy_from_slice(vault_1.as_ref());
    d[201..233].copy_from_slice(observation.as_ref());
    d[233] = DECIMALS;
    d[234] = DECIMALS;
    d[235..237].copy_from_slice(&60u16.to_le_bytes());
    d[237..253].copy_from_slice(&1_000_000_000u128.to_le_bytes());
    d[253..269].copy_from_slice(&(1u128 << 64).to_le_bytes());
    d[269..273].copy_from_slice(&0i32.to_le_bytes());
    d
}

fn set_mint(svm: &mut LiteSVM, mint: &Pubkey, decimals: u8, supply: u64, authority: &Pubkey) {
    svm.set_account(
        mint.to_bytes().into(),
        Account {
            lamports: 10_000_000,
            data: mint_bytes(decimals, supply, authority),
            owner: token_program().to_bytes().into(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn set_token_account(svm: &mut LiteSVM, address: &Pubkey, mint: &Pubkey, owner: &Pubkey, amount: u64) {
    svm.set_account(
        address.to_bytes().into(),
        Account {
            lamports: 10_000_000,
            data: token_account_bytes(mint, owner, amount),
            owner: token_program().to_bytes().into(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn read_token_amount(svm: &LiteSVM, address: &Pubkey) -> u64 {
    let acc = svm.get_account(&address.to_bytes().into()).unwrap();
    u64::from_le_bytes(acc.data[64..72].try_into().unwrap())
}

fn read_mint_supply(svm: &LiteSVM, mint: &Pubkey) -> u64 {
    let acc = svm.get_account(&mint.to_bytes().into()).unwrap();
    u64::from_le_bytes(acc.data[36..44].try_into().unwrap())
}

fn setup_svm() -> (LiteSVM, Keypair, Pubkey, Pubkey) {
    let mut svm = LiteSVM::new();
    let so_path = format!(
        "{}/../../target/deploy/liquidity_vault.so",
        env!("CARGO_MANIFEST_DIR")
    );
    let program_bytes = std::fs::read(&so_path)
        .unwrap_or_else(|_| panic!("Run `cargo build-sbf` before tests (missing {})", so_path));
    svm.add_program(liquidity_vault::id().to_bytes(), &program_bytes)
        .unwrap();

    let admin = Keypair::new();
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    svm.airdrop(&admin.pubkey(), 20 * SOL).unwrap();

    let deposit_mint = Pubkey::new_unique();
    let secondary_mint = Pubkey::new_unique();
    set_mint(&mut svm, &deposit_mint, DECIMALS, 0, &admin_pk);
    set_mint(&mut svm, &secondary_mint, DECIMALS, 0, &admin_pk);

    (svm, admin, deposit_mint, secondary_mint)
}

fn send_ix(svm: &mut LiteSVM, signers: &[&Keypair], ix: Instruction) -> Result<(), String> {
    let payer = signers[0].pubkey();
    let bh = svm.latest_blockhash();
    let msg = Message::new(&[ix], Some(&payer));
    let tx = Transaction::new(signers, msg, bh);
    svm.send_transaction(tx)
        .map(|_| ())
        .map_err(|e| format!("{:?}", e.err))
}

fn init_vault(svm: &mut LiteSVM, admin: &Keypair, deposit_mint: &Pubkey) -> (Pubkey, Pubkey, Pubkey) {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let vault = pda(&[VAULT_SEED, deposit_mint.as_ref()]);
    let token_vault = pda(&[VAULT_TOKEN_SEED, vault.as_ref()]);
    let share_mint = pda(&[SHARE_MINT_SEED, vault.as_ref()]);

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::InitializeVault {
            admin: admin_pk,
            deposit_mint: *deposit_mint,
            vault,
            token_vault,
            share_mint,
            token_program: token_program(),
            system_program: system_program::ID,
            rent: rent_sysvar(),
        }
        .to_account_metas(None),
        data: instruction::InitializeVault {}.data(),
    };
    send_ix(svm, &[admin], ix).unwrap();
    (vault, token_vault, share_mint)
}

fn create_card(svm: &mut LiteSVM, user: &Keypair, vault: &Pubkey) -> Pubkey {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let user_card = pda(&[USER_CARD_SEED, vault.as_ref(), user_pk.as_ref()]);
    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::CreateUserCard {
            user: user_pk,
            vault: *vault,
            user_card,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::CreateUserCard {}.data(),
    };
    send_ix(svm, &[user], ix).unwrap();
    user_card
}

#[test]
fn week8_initialize_vault_and_create_user_cards() {
    let (mut svm, admin, deposit_mint, _) = setup_svm();
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, token_vault, share_mint) = init_vault(&mut svm, &admin, &deposit_mint);

    let vault_acc = svm.get_account(&vault.to_bytes().into()).unwrap();
    let mut slice: &[u8] = &vault_acc.data;
    let state = VaultState::try_deserialize(&mut slice).unwrap();
    assert_eq!(state.admin, admin_pk);
    assert_eq!(state.deposit_mint, deposit_mint);
    assert_eq!(state.token_vault, token_vault);
    assert_eq!(state.share_mint, share_mint);
    assert_eq!(state.total_deposited, 0);
    assert_eq!(state.total_shares, 0);

    // Карточки двух разных пользователей имеют независимые PDA
    let user_a = Keypair::new();
    let user_b = Keypair::new();
    svm.airdrop(&user_a.pubkey(), 5 * SOL).unwrap();
    svm.airdrop(&user_b.pubkey(), 5 * SOL).unwrap();

    let card_a = create_card(&mut svm, &user_a, &vault);
    let card_b = create_card(&mut svm, &user_b, &vault);
    assert_ne!(card_a, card_b);

    let card_a_acc = svm.get_account(&card_a.to_bytes().into()).unwrap();
    let mut card_slice: &[u8] = &card_a_acc.data;
    let parsed_card = UserCard::try_deserialize(&mut card_slice).unwrap();
    assert_eq!(parsed_card.vault, vault);
    assert_eq!(
        parsed_card.owner,
        Pubkey::new_from_array(user_a.pubkey().to_bytes())
    );
    assert_eq!(parsed_card.deposited_amount, 0);
    assert_eq!(parsed_card.shares_minted, 0);
}

#[test]
fn week9_deposit_share_calculation_and_zero_rejections() {
    let (mut svm, admin, deposit_mint, _) = setup_svm();
    let (vault, token_vault, share_mint) = init_vault(&mut svm, &admin, &deposit_mint);

    let user1 = Keypair::new();
    let user2 = Keypair::new();
    svm.airdrop(&user1.pubkey(), 5 * SOL).unwrap();
    svm.airdrop(&user2.pubkey(), 5 * SOL).unwrap();
    let u1_pk = Pubkey::new_from_array(user1.pubkey().to_bytes());
    let u2_pk = Pubkey::new_from_array(user2.pubkey().to_bytes());

    let card1 = create_card(&mut svm, &user1, &vault);
    let card2 = create_card(&mut svm, &user2, &vault);

    let u1_token = ata(&u1_pk, &deposit_mint);
    let u2_token = ata(&u2_pk, &deposit_mint);
    let u1_share = ata(&u1_pk, &share_mint);
    let u2_share = ata(&u2_pk, &share_mint);

    set_token_account(&mut svm, &u1_token, &deposit_mint, &u1_pk, 2_000 * ONE_TOKEN);
    set_token_account(&mut svm, &u2_token, &deposit_mint, &u2_pk, 500 * ONE_TOKEN);

    // Отказ при попытке внести 0 токенов
    let zero_ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::Deposit {
            user: u1_pk,
            vault,
            user_card: card1,
            deposit_mint,
            share_mint,
            user_token_account: u1_token,
            token_vault,
            user_share_account: u1_share,
            token_program: token_program(),
            associated_token_program: ata_program(),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::Deposit { amount: 0 }.data(),
    };
    let err = send_ix(&mut svm, &[&user1], zero_ix).unwrap_err();
    assert!(err.contains(&custom_err(VaultError::ZeroAmount)), "got {}", err);

    // Первый вкладчик вносит 1000 токенов -> получает ровно 1000 share-токенов (1:1)
    let dep1_ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::Deposit {
            user: u1_pk,
            vault,
            user_card: card1,
            deposit_mint,
            share_mint,
            user_token_account: u1_token,
            token_vault,
            user_share_account: u1_share,
            token_program: token_program(),
            associated_token_program: ata_program(),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::Deposit {
            amount: 1_000 * ONE_TOKEN,
        }
        .data(),
    };
    send_ix(&mut svm, &[&user1], dep1_ix).unwrap();
    assert_eq!(read_token_amount(&svm, &u1_share), 1_000 * ONE_TOKEN);
    assert_eq!(read_token_amount(&svm, &token_vault), 1_000 * ONE_TOKEN);

    // Vault зарабатывает +100 токенов дохода: в хранилище становится 1100 токенов при 1000 долях
    set_token_account(
        &mut svm,
        &token_vault,
        &deposit_mint,
        &vault,
        1_100 * ONE_TOKEN,
    );

    // Второй вкладчик вносит 110 токенов -> получает 110 * 1000 / 1100 = 100 share-токенов
    let dep2_ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::Deposit {
            user: u2_pk,
            vault,
            user_card: card2,
            deposit_mint,
            share_mint,
            user_token_account: u2_token,
            token_vault,
            user_share_account: u2_share,
            token_program: token_program(),
            associated_token_program: ata_program(),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::Deposit {
            amount: 110 * ONE_TOKEN,
        }
        .data(),
    };
    send_ix(&mut svm, &[&user2], dep2_ix).unwrap();
    assert_eq!(read_token_amount(&svm, &u2_share), 100 * ONE_TOKEN);
    assert_eq!(read_token_amount(&svm, &token_vault), 1_210 * ONE_TOKEN);
    assert_eq!(read_mint_supply(&svm, &share_mint), 1_100 * ONE_TOKEN);

    // Пылевой депозит (1 микротокен при курсе 1.1) округляется в 0 долей и отклоняется ZeroShares
    let dust_ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::Deposit {
            user: u2_pk,
            vault,
            user_card: card2,
            deposit_mint,
            share_mint,
            user_token_account: u2_token,
            token_vault,
            user_share_account: u2_share,
            token_program: token_program(),
            associated_token_program: ata_program(),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::Deposit { amount: 1 }.data(),
    };
    let dust_err = send_ix(&mut svm, &[&user2], dust_ix).unwrap_err();
    assert!(
        dust_err.contains(&custom_err(VaultError::ZeroShares)),
        "got {}",
        dust_err
    );
}

#[test]
fn week10_withdraw_cpi_yield_distribution_and_error_checks() {
    let (mut svm, admin, deposit_mint, _) = setup_svm();
    let (vault, token_vault, share_mint) = init_vault(&mut svm, &admin, &deposit_mint);

    let user1 = Keypair::new();
    let user2 = Keypair::new();
    svm.airdrop(&user1.pubkey(), 5 * SOL).unwrap();
    svm.airdrop(&user2.pubkey(), 5 * SOL).unwrap();
    let u1_pk = Pubkey::new_from_array(user1.pubkey().to_bytes());
    let u2_pk = Pubkey::new_from_array(user2.pubkey().to_bytes());

    let card1 = create_card(&mut svm, &user1, &vault);
    let card2 = create_card(&mut svm, &user2, &vault);

    let u1_token = ata(&u1_pk, &deposit_mint);
    let u2_token = ata(&u2_pk, &deposit_mint);
    let u1_share = ata(&u1_pk, &share_mint);
    let u2_share = ata(&u2_pk, &share_mint);

    set_token_account(&mut svm, &u1_token, &deposit_mint, &u1_pk, 1_000 * ONE_TOKEN);
    set_token_account(&mut svm, &u2_token, &deposit_mint, &u2_pk, 110 * ONE_TOKEN);

    // 1. User 1 вносит 1000 токенов -> получает 1000 share-токенов
    send_ix(
        &mut svm,
        &[&user1],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::Deposit {
                user: u1_pk,
                vault,
                user_card: card1,
                deposit_mint,
                share_mint,
                user_token_account: u1_token,
                token_vault,
                user_share_account: u1_share,
                token_program: token_program(),
                associated_token_program: ata_program(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::Deposit {
                amount: 1_000 * ONE_TOKEN,
            }
            .data(),
        },
    )
    .unwrap();

    // 2. Vault зарабатывает +100 токенов (в хранилище 1100 токенов)
    set_token_account(
        &mut svm,
        &token_vault,
        &deposit_mint,
        &vault,
        1_100 * ONE_TOKEN,
    );

    // 3. User 2 вносит 110 токенов -> получает 100 share-токенов
    send_ix(
        &mut svm,
        &[&user2],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::Deposit {
                user: u2_pk,
                vault,
                user_card: card2,
                deposit_mint,
                share_mint,
                user_token_account: u2_token,
                token_vault,
                user_share_account: u2_share,
                token_program: token_program(),
                associated_token_program: ata_program(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::Deposit {
                amount: 110 * ONE_TOKEN,
            }
            .data(),
        },
    )
    .unwrap();

    // Попытка вывести больше долей, чем есть на балансе -> InsufficientShares
    let excess_err = send_ix(
        &mut svm,
        &[&user2],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::Withdraw {
                user: u2_pk,
                vault,
                user_card: card2,
                deposit_mint,
                share_mint,
                user_token_account: u2_token,
                token_vault,
                user_share_account: u2_share,
                token_program: token_program(),
            }
            .to_account_metas(None),
            data: instruction::Withdraw {
                shares: 200 * ONE_TOKEN,
            }
            .data(),
        },
    )
    .unwrap_err();
    assert!(
        excess_err.contains(&custom_err(VaultError::InsufficientShares)),
        "got {}",
        excess_err
    );

    // 4. User 1 сжигает все 1000 share-токенов -> получает 1100 базовых токенов (свой вклад + доход)
    send_ix(
        &mut svm,
        &[&user1],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::Withdraw {
                user: u1_pk,
                vault,
                user_card: card1,
                deposit_mint,
                share_mint,
                user_token_account: u1_token,
                token_vault,
                user_share_account: u1_share,
                token_program: token_program(),
            }
            .to_account_metas(None),
            data: instruction::Withdraw {
                shares: 1_000 * ONE_TOKEN,
            }
            .data(),
        },
    )
    .unwrap();
    assert_eq!(read_token_amount(&svm, &u1_token), 1_100 * ONE_TOKEN);
    assert_eq!(read_token_amount(&svm, &u1_share), 0);

    // 5. User 2 сжигает свои 100 share-токенов -> получает ровно 110 базовых токенов
    send_ix(
        &mut svm,
        &[&user2],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::Withdraw {
                user: u2_pk,
                vault,
                user_card: card2,
                deposit_mint,
                share_mint,
                user_token_account: u2_token,
                token_vault,
                user_share_account: u2_share,
                token_program: token_program(),
            }
            .to_account_metas(None),
            data: instruction::Withdraw {
                shares: 100 * ONE_TOKEN,
            }
            .data(),
        },
    )
    .unwrap();
    assert_eq!(read_token_amount(&svm, &u2_token), 110 * ONE_TOKEN);
    assert_eq!(read_token_amount(&svm, &u2_share), 0);
    assert_eq!(read_token_amount(&svm, &token_vault), 0);
    assert_eq!(read_mint_supply(&svm, &share_mint), 0);
}

#[test]
fn week12_and_week13_strategy_swap_and_position_guards() {
    let (mut svm, admin, deposit_mint, secondary_mint) = setup_svm();
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, token_vault, _) = init_vault(&mut svm, &admin, &deposit_mint);

    let clmm_program = mock_clmm_program();
    let so_path = format!(
        "{}/../../target/deploy/liquidity_vault.so",
        env!("CARGO_MANIFEST_DIR")
    );
    let program_bytes = std::fs::read(&so_path).unwrap();
    svm.add_program(clmm_program.to_bytes(), &program_bytes)
        .unwrap();

    let amm_config = Pubkey::new_unique();
    let pool_state = Pubkey::new_unique();
    let pool_vault_0 = Pubkey::new_unique();
    let pool_vault_1 = Pubkey::new_unique();
    let observation = Pubkey::new_unique();

    svm.set_account(
        pool_state.to_bytes().into(),
        Account {
            lamports: 10 * SOL,
            data: mock_pool_state_bytes(
                &amm_config,
                &deposit_mint,
                &secondary_mint,
                &pool_vault_0,
                &pool_vault_1,
                &observation,
            ),
            owner: clmm_program.to_bytes().into(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();

    let strategy = pda(&[STRATEGY_SEED, vault.as_ref()]);
    let secondary_vault = pda(&[STRATEGY_VAULT_SEED, vault.as_ref()]);

    // Посторонний пользователь не может инициализировать стратегию
    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), 5 * SOL).unwrap();
    let attacker_pk = Pubkey::new_from_array(attacker.pubkey().to_bytes());
    let bad_init_err = send_ix(
        &mut svm,
        &[&attacker],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::InitStrategy {
                admin: attacker_pk,
                vault,
                strategy,
                secondary_mint,
                secondary_vault,
                clmm_program,
                amm_config,
                pool_state,
                token_program: token_program(),
                system_program: system_program::ID,
                rent: rent_sysvar(),
            }
            .to_account_metas(None),
            data: instruction::InitStrategy {}.data(),
        },
    )
    .unwrap_err();
    assert!(
        bad_init_err.contains(&custom_err(VaultError::Unauthorized)),
        "got {}",
        bad_init_err
    );

    // Администратор успешно инициализирует стратегию и второе хранилище
    send_ix(
        &mut svm,
        &[&admin],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::InitStrategy {
                admin: admin_pk,
                vault,
                strategy,
                secondary_mint,
                secondary_vault,
                clmm_program,
                amm_config,
                pool_state,
                token_program: token_program(),
                system_program: system_program::ID,
                rent: rent_sysvar(),
            }
            .to_account_metas(None),
            data: instruction::InitStrategy {}.data(),
        },
    )
    .unwrap();

    let strat_acc = svm.get_account(&strategy.to_bytes().into()).unwrap();
    let mut slice: &[u8] = &strat_acc.data;
    let parsed_strat = Strategy::try_deserialize(&mut slice).unwrap();
    assert_eq!(parsed_strat.vault, vault);
    assert_eq!(parsed_strat.pool_state, pool_state);
    assert_eq!(parsed_strat.secondary_mint, secondary_mint);
    assert_eq!(parsed_strat.secondary_vault, secondary_vault);
    assert!(!parsed_strat.active_position);

    // Проверка защит swap: отказ не-админу и отказ при подмене хранилищ пула
    set_token_account(
        &mut svm,
        &token_vault,
        &deposit_mint,
        &vault,
        500 * ONE_TOKEN,
    );
    let wrong_vault_err = send_ix(
        &mut svm,
        &[&admin],
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::SwapVault {
                admin: admin_pk,
                vault,
                strategy,
                token_vault,
                secondary_vault,
                deposit_mint,
                secondary_mint,
                clmm_program,
                amm_config,
                pool_state,
                input_vault: pool_vault_1, // намеренно перепутаны хранилища пула
                output_vault: pool_vault_0,
                observation_state: observation,
                token_program: token_program(),
                token_program_2022: token_2022_program(),
                memo_program: memo_program(),
            }
            .to_account_metas(None),
            data: instruction::Swap {
                amount_in: 100 * ONE_TOKEN,
                min_amount_out: 95 * ONE_TOKEN,
                sqrt_price_limit_x64: 0,
                a_to_b: true,
            }
            .data(),
        },
    )
    .unwrap_err();
    assert!(
        wrong_vault_err.contains(&custom_err(VaultError::InvalidPoolVaults)),
        "got {}",
        wrong_vault_err
    );
}
