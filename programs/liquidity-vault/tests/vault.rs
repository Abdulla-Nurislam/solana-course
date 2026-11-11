use std::str::FromStr;

use anchor_lang::{
    prelude::Pubkey,
    solana_program::instruction::Instruction,
    system_program, AccountDeserialize, InstructionData, ToAccountMetas,
};
use litesvm::LiteSVM;
use liquidity_vault::{
    accounts, instruction,
    state::{
        UserCard, VaultState, SHARE_MINT_SEED, USER_CARD_SEED, VAULT_SEED, VAULT_TOKEN_SEED,
    },
};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

const DECIMALS: u8 = 6;
const SOL: u64 = 1_000_000_000;

fn pk(s: &str) -> Pubkey {
    Pubkey::from_str(s).unwrap()
}

fn token_program() -> Pubkey {
    pk("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
}

fn rent_sysvar() -> Pubkey {
    pk("SysvarRent111111111111111111111111111111111")
}

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &liquidity_vault::id()).0
}

fn setup_svm() -> (LiteSVM, Keypair, Pubkey) {
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
    svm.airdrop(&admin.pubkey(), 20 * SOL).unwrap();
    let deposit_mint = Pubkey::new_unique();
    (svm, admin, deposit_mint)
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
    let (mut svm, admin, deposit_mint) = setup_svm();
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let _ = DECIMALS;
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
}
