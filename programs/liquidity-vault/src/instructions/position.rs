use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_2022::Token2022,
    token_interface::{self, Approve, Mint, Revoke, TokenAccount, TokenInterface},
};

use crate::{
    error::VaultError,
    raydium::{
        invoke_close_position, invoke_decrease_liquidity_v2, invoke_increase_liquidity_v2,
        invoke_open_position_with_token22_nft, parse_personal_position, parse_pool_state,
    },
    state::{Strategy, VaultState, STRATEGY_SEED, VAULT_SEED},
};

#[derive(Accounts)]
pub struct OpenVaultPosition<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED, vault.deposit_mint.as_ref()],
        bump = vault.bump,
        has_one = admin @ VaultError::Unauthorized,
        has_one = token_vault @ VaultError::InvalidVaultTokenAccount
    )]
    pub vault: Box<Account<'info, VaultState>>,

    #[account(
        mut,
        seeds = [STRATEGY_SEED, vault.key().as_ref()],
        bump = strategy.bump,
        has_one = vault @ VaultError::Unauthorized,
        has_one = secondary_vault @ VaultError::InvalidVaultTokenAccount,
        has_one = clmm_program @ VaultError::InvalidClmmProgram,
        has_one = pool_state @ VaultError::InvalidPoolState
    )]
    pub strategy: Box<Account<'info, Strategy>>,

    #[account(mut)]
    pub position_nft_mint: Signer<'info>,

    /// CHECK: Создаётся внутри CPI `open_position_with_token22_nft` (Token-2022 ATA `vault` PDA).
    #[account(mut)]
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: Аккаунт `PoolState` Raydium CLMM.
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: Аккаунт `ProtocolPositionState` Raydium CLMM.
    #[account(mut)]
    pub protocol_position: UncheckedAccount<'info>,

    /// CHECK: Нижний `TickArrayState` пула Raydium CLMM.
    #[account(mut)]
    pub tick_array_lower: UncheckedAccount<'info>,

    /// CHECK: Верхний `TickArrayState` пула Raydium CLMM.
    #[account(mut)]
    pub tick_array_upper: UncheckedAccount<'info>,

    /// CHECK: Аккаунт `PersonalPositionState` Raydium CLMM (создаётся в CPI).
    #[account(mut)]
    pub personal_position: UncheckedAccount<'info>,

    #[account(
        mut,
        token::mint = deposit_mint,
        token::authority = vault
    )]
    pub token_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = secondary_mint,
        token::authority = vault
    )]
    pub secondary_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: Хранилище `token_vault_0` пула Raydium CLMM.
    #[account(mut)]
    pub pool_vault_0: UncheckedAccount<'info>,

    /// CHECK: Хранилище `token_vault_1` пула Raydium CLMM.
    #[account(mut)]
    pub pool_vault_1: UncheckedAccount<'info>,

    #[account(address = vault.deposit_mint @ VaultError::InvalidPoolMints)]
    pub deposit_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(address = strategy.secondary_mint @ VaultError::InvalidPoolMints)]
    pub secondary_mint: Box<InterfaceAccount<'info, Mint>>,

    /// CHECK: Проверено через `has_one = clmm_program` в `strategy`.
    #[account(constraint = clmm_program.executable @ VaultError::InvalidClmmProgram)]
    pub clmm_program: UncheckedAccount<'info>,

    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program_2022: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct CollectVaultFees<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED, vault.deposit_mint.as_ref()],
        bump = vault.bump,
        has_one = admin @ VaultError::Unauthorized,
        has_one = token_vault @ VaultError::InvalidVaultTokenAccount
    )]
    pub vault: Box<Account<'info, VaultState>>,

    #[account(
        mut,
        seeds = [STRATEGY_SEED, vault.key().as_ref()],
        bump = strategy.bump,
        has_one = vault @ VaultError::Unauthorized,
        has_one = secondary_vault @ VaultError::InvalidVaultTokenAccount,
        has_one = clmm_program @ VaultError::InvalidClmmProgram,
        has_one = pool_state @ VaultError::InvalidPoolState
    )]
    pub strategy: Box<Account<'info, Strategy>>,

    /// CHECK: Token-2022 ATA NFT позиции, принадлежащий `vault` PDA.
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: `PersonalPositionState` позиции `vault` в Raydium CLMM.
    #[account(mut)]
    pub personal_position: UncheckedAccount<'info>,

    /// CHECK: `PoolState` Raydium CLMM.
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: `ProtocolPositionState` Raydium CLMM.
    #[account(mut)]
    pub protocol_position: UncheckedAccount<'info>,

    /// CHECK: `token_vault_0` пула Raydium CLMM.
    #[account(mut)]
    pub pool_vault_0: UncheckedAccount<'info>,

    /// CHECK: `token_vault_1` пула Raydium CLMM.
    #[account(mut)]
    pub pool_vault_1: UncheckedAccount<'info>,

    /// CHECK: Нижний `TickArrayState` пула Raydium CLMM.
    #[account(mut)]
    pub tick_array_lower: UncheckedAccount<'info>,

    /// CHECK: Верхний `TickArrayState` пула Raydium CLMM.
    #[account(mut)]
    pub tick_array_upper: UncheckedAccount<'info>,

    #[account(
        mut,
        token::mint = deposit_mint,
        token::authority = vault
    )]
    pub token_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = secondary_mint,
        token::authority = vault
    )]
    pub secondary_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(address = vault.deposit_mint @ VaultError::InvalidPoolMints)]
    pub deposit_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(address = strategy.secondary_mint @ VaultError::InvalidPoolMints)]
    pub secondary_mint: Box<InterfaceAccount<'info, Mint>>,

    /// CHECK: Программа Raydium CLMM.
    #[account(constraint = clmm_program.executable @ VaultError::InvalidClmmProgram)]
    pub clmm_program: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub token_program_2022: Program<'info, Token2022>,

    /// CHECK: SPL Memo Program.
    pub memo_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct CloseVaultPosition<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED, vault.deposit_mint.as_ref()],
        bump = vault.bump,
        has_one = admin @ VaultError::Unauthorized,
        has_one = token_vault @ VaultError::InvalidVaultTokenAccount
    )]
    pub vault: Box<Account<'info, VaultState>>,

    #[account(
        mut,
        seeds = [STRATEGY_SEED, vault.key().as_ref()],
        bump = strategy.bump,
        has_one = vault @ VaultError::Unauthorized,
        has_one = secondary_vault @ VaultError::InvalidVaultTokenAccount,
        has_one = clmm_program @ VaultError::InvalidClmmProgram,
        has_one = pool_state @ VaultError::InvalidPoolState
    )]
    pub strategy: Box<Account<'info, Strategy>>,

    /// CHECK: Минт Token-2022 NFT позиции.
    #[account(mut)]
    pub position_nft_mint: UncheckedAccount<'info>,

    /// CHECK: Token-2022 ATA NFT позиции, принадлежащий `vault` PDA.
    #[account(mut)]
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: `PersonalPositionState` позиции `vault` в Raydium CLMM.
    #[account(mut)]
    pub personal_position: UncheckedAccount<'info>,

    /// CHECK: `PoolState` Raydium CLMM.
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: `ProtocolPositionState` Raydium CLMM.
    #[account(mut)]
    pub protocol_position: UncheckedAccount<'info>,

    /// CHECK: `token_vault_0` пула Raydium CLMM.
    #[account(mut)]
    pub pool_vault_0: UncheckedAccount<'info>,

    /// CHECK: `token_vault_1` пула Raydium CLMM.
    #[account(mut)]
    pub pool_vault_1: UncheckedAccount<'info>,

    /// CHECK: Нижний `TickArrayState` пула Raydium CLMM.
    #[account(mut)]
    pub tick_array_lower: UncheckedAccount<'info>,

    /// CHECK: Верхний `TickArrayState` пула Raydium CLMM.
    #[account(mut)]
    pub tick_array_upper: UncheckedAccount<'info>,

    #[account(
        mut,
        token::mint = deposit_mint,
        token::authority = vault
    )]
    pub token_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = secondary_mint,
        token::authority = vault
    )]
    pub secondary_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(address = vault.deposit_mint @ VaultError::InvalidPoolMints)]
    pub deposit_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(address = strategy.secondary_mint @ VaultError::InvalidPoolMints)]
    pub secondary_mint: Box<InterfaceAccount<'info, Mint>>,

    /// CHECK: Программа Raydium CLMM.
    #[account(constraint = clmm_program.executable @ VaultError::InvalidClmmProgram)]
    pub clmm_program: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub token_program_2022: Program<'info, Token2022>,

    /// CHECK: SPL Memo Program.
    pub memo_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

fn ordered_vault_accounts<'info>(
    deposit_is_token_0: bool,
    token_vault: &InterfaceAccount<'info, TokenAccount>,
    secondary_vault: &InterfaceAccount<'info, TokenAccount>,
    deposit_mint: &InterfaceAccount<'info, Mint>,
    secondary_mint: &InterfaceAccount<'info, Mint>,
) -> (
    AccountInfo<'info>,
    AccountInfo<'info>,
    AccountInfo<'info>,
    AccountInfo<'info>,
) {
    if deposit_is_token_0 {
        (
            token_vault.to_account_info(),
            secondary_vault.to_account_info(),
            deposit_mint.to_account_info(),
            secondary_mint.to_account_info(),
        )
    } else {
        (
            secondary_vault.to_account_info(),
            token_vault.to_account_info(),
            secondary_mint.to_account_info(),
            deposit_mint.to_account_info(),
        )
    }
}

pub fn open_position<'info>(
    ctx: Context<'info, OpenVaultPosition<'info>>,
    tick_lower_index: i32,
    tick_upper_index: i32,
    tick_array_lower_start_index: i32,
    tick_array_upper_start_index: i32,
    liquidity: u128,
    amount_0_max: u64,
    amount_1_max: u64,
) -> Result<()> {
    require!(
        !ctx.accounts.strategy.active_position,
        VaultError::PositionAlreadyActive
    );
    require!(
        tick_lower_index < tick_upper_index,
        VaultError::InvalidTickRange
    );
    require!(liquidity > 0, VaultError::ZeroAmount);
    require!(
        amount_0_max > 0 || amount_1_max > 0,
        VaultError::ZeroAmount
    );

    let parsed_pool = parse_pool_state(
        &ctx.accounts.pool_state.to_account_info(),
        &ctx.accounts.clmm_program.key(),
    )?;
    require_keys_eq!(
        parsed_pool.token_vault_0,
        ctx.accounts.pool_vault_0.key(),
        VaultError::InvalidPoolVaults
    );
    require_keys_eq!(
        parsed_pool.token_vault_1,
        ctx.accounts.pool_vault_1.key(),
        VaultError::InvalidPoolVaults
    );

    let deposit_is_token_0 = parsed_pool.token_mint_0 == ctx.accounts.vault.deposit_mint;
    let (token_account_0, token_account_1, vault_0_mint, vault_1_mint) = ordered_vault_accounts(
        deposit_is_token_0,
        &ctx.accounts.token_vault,
        &ctx.accounts.secondary_vault,
        &ctx.accounts.deposit_mint,
        &ctx.accounts.secondary_mint,
    );

    let deposit_mint_key = ctx.accounts.vault.deposit_mint;
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_SEED,
        deposit_mint_key.as_ref(),
        &[ctx.accounts.vault.bump],
    ]];

    // Временно делегируем `admin` право списания в пределах `amount_0_max` / `amount_1_max`,
    // так как `open_position_with_token22_nft` создаёт аккаунты за счёт `payer = admin`
    // и списывает ликвидность подписью `payer`, а владельцем NFT-позиции делает `vault` PDA.
    if amount_0_max > 0 {
        token_interface::approve(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Approve {
                    to: token_account_0.clone(),
                    delegate: ctx.accounts.admin.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount_0_max,
        )?;
    }
    if amount_1_max > 0 {
        token_interface::approve(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Approve {
                    to: token_account_1.clone(),
                    delegate: ctx.accounts.admin.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount_1_max,
        )?;
    }

    invoke_open_position_with_token22_nft(
        &ctx.accounts.clmm_program.to_account_info(),
        &ctx.accounts.admin.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.position_nft_mint.to_account_info(),
        &ctx.accounts.position_nft_account.to_account_info(),
        &ctx.accounts.pool_state.to_account_info(),
        &ctx.accounts.protocol_position.to_account_info(),
        &ctx.accounts.tick_array_lower.to_account_info(),
        &ctx.accounts.tick_array_upper.to_account_info(),
        &ctx.accounts.personal_position.to_account_info(),
        &token_account_0,
        &token_account_1,
        &ctx.accounts.pool_vault_0.to_account_info(),
        &ctx.accounts.pool_vault_1.to_account_info(),
        &ctx.accounts.rent.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.associated_token_program.to_account_info(),
        &ctx.accounts.token_program_2022.to_account_info(),
        &vault_0_mint,
        &vault_1_mint,
        ctx.remaining_accounts,
        tick_lower_index,
        tick_upper_index,
        tick_array_lower_start_index,
        tick_array_upper_start_index,
        liquidity,
        amount_0_max,
        amount_1_max,
        false,
        signer_seeds,
    )?;

    // Немедленно отзываем делегирование с обоих хранилищ в той же инструкции.
    if amount_0_max > 0 {
        token_interface::revoke(CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Revoke {
                source: token_account_0,
                authority: ctx.accounts.vault.to_account_info(),
            },
            signer_seeds,
        ))?;
    }
    if amount_1_max > 0 {
        token_interface::revoke(CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Revoke {
                source: token_account_1,
                authority: ctx.accounts.vault.to_account_info(),
            },
            signer_seeds,
        ))?;
    }

    let parsed_pos = parse_personal_position(
        &ctx.accounts.personal_position.to_account_info(),
        &ctx.accounts.clmm_program.key(),
    )?;

    let strategy = &mut ctx.accounts.strategy;
    strategy.position_nft_mint = ctx.accounts.position_nft_mint.key();
    strategy.position_nft_account = ctx.accounts.position_nft_account.key();
    strategy.personal_position = ctx.accounts.personal_position.key();
    strategy.tick_lower_index = tick_lower_index;
    strategy.tick_upper_index = tick_upper_index;
    strategy.liquidity = parsed_pos.liquidity;
    strategy.active_position = true;

    Ok(())
}

/// Вспомогательное пополнение уже открытой позиции через `increase_liquidity_v2` подписью `vault` PDA.
#[allow(clippy::too_many_arguments)]
pub fn increase_existing_position<'info>(
    clmm_program: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    position_nft_account: &AccountInfo<'info>,
    pool_state: &AccountInfo<'info>,
    protocol_position: &AccountInfo<'info>,
    personal_position: &AccountInfo<'info>,
    tick_array_lower: &AccountInfo<'info>,
    tick_array_upper: &AccountInfo<'info>,
    token_account_0: &AccountInfo<'info>,
    token_account_1: &AccountInfo<'info>,
    pool_vault_0: &AccountInfo<'info>,
    pool_vault_1: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    token_program_2022: &AccountInfo<'info>,
    vault_0_mint: &AccountInfo<'info>,
    vault_1_mint: &AccountInfo<'info>,
    liquidity: u128,
    amount_0_max: u64,
    amount_1_max: u64,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    invoke_increase_liquidity_v2(
        clmm_program,
        vault,
        position_nft_account,
        pool_state,
        protocol_position,
        personal_position,
        tick_array_lower,
        tick_array_upper,
        token_account_0,
        token_account_1,
        pool_vault_0,
        pool_vault_1,
        token_program,
        token_program_2022,
        vault_0_mint,
        vault_1_mint,
        liquidity,
        amount_0_max,
        amount_1_max,
        signer_seeds,
    )
}

pub fn collect_fees<'info>(
    ctx: Context<'info, CollectVaultFees<'info>>,
) -> Result<()> {
    require!(
        ctx.accounts.strategy.active_position,
        VaultError::NoActivePosition
    );
    require_keys_eq!(
        ctx.accounts.strategy.position_nft_account,
        ctx.accounts.position_nft_account.key(),
        VaultError::InvalidPersonalPosition
    );
    require_keys_eq!(
        ctx.accounts.strategy.personal_position,
        ctx.accounts.personal_position.key(),
        VaultError::InvalidPersonalPosition
    );

    let parsed_pool = parse_pool_state(
        &ctx.accounts.pool_state.to_account_info(),
        &ctx.accounts.clmm_program.key(),
    )?;
    require_keys_eq!(
        parsed_pool.token_vault_0,
        ctx.accounts.pool_vault_0.key(),
        VaultError::InvalidPoolVaults
    );
    require_keys_eq!(
        parsed_pool.token_vault_1,
        ctx.accounts.pool_vault_1.key(),
        VaultError::InvalidPoolVaults
    );

    let deposit_is_token_0 = parsed_pool.token_mint_0 == ctx.accounts.vault.deposit_mint;
    let (recipient_0, recipient_1, vault_0_mint, vault_1_mint) = ordered_vault_accounts(
        deposit_is_token_0,
        &ctx.accounts.token_vault,
        &ctx.accounts.secondary_vault,
        &ctx.accounts.deposit_mint,
        &ctx.accounts.secondary_mint,
    );

    let deposit_mint_key = ctx.accounts.vault.deposit_mint;
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_SEED,
        deposit_mint_key.as_ref(),
        &[ctx.accounts.vault.bump],
    ]];

    invoke_decrease_liquidity_v2(
        &ctx.accounts.clmm_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.position_nft_account.to_account_info(),
        &ctx.accounts.personal_position.to_account_info(),
        &ctx.accounts.pool_state.to_account_info(),
        &ctx.accounts.protocol_position.to_account_info(),
        &ctx.accounts.pool_vault_0.to_account_info(),
        &ctx.accounts.pool_vault_1.to_account_info(),
        &ctx.accounts.tick_array_lower.to_account_info(),
        &ctx.accounts.tick_array_upper.to_account_info(),
        &recipient_0,
        &recipient_1,
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.token_program_2022.to_account_info(),
        &ctx.accounts.memo_program.to_account_info(),
        &vault_0_mint,
        &vault_1_mint,
        ctx.remaining_accounts,
        0,
        0,
        0,
        signer_seeds,
    )?;

    Ok(())
}

pub fn close_position<'info>(
    ctx: Context<'info, CloseVaultPosition<'info>>,
    amount_0_min: u64,
    amount_1_min: u64,
) -> Result<()> {
    require!(
        ctx.accounts.strategy.active_position,
        VaultError::NoActivePosition
    );
    require_keys_eq!(
        ctx.accounts.strategy.position_nft_mint,
        ctx.accounts.position_nft_mint.key(),
        VaultError::InvalidPersonalPosition
    );
    require_keys_eq!(
        ctx.accounts.strategy.position_nft_account,
        ctx.accounts.position_nft_account.key(),
        VaultError::InvalidPersonalPosition
    );
    require_keys_eq!(
        ctx.accounts.strategy.personal_position,
        ctx.accounts.personal_position.key(),
        VaultError::InvalidPersonalPosition
    );

    let parsed_pool = parse_pool_state(
        &ctx.accounts.pool_state.to_account_info(),
        &ctx.accounts.clmm_program.key(),
    )?;
    require_keys_eq!(
        parsed_pool.token_vault_0,
        ctx.accounts.pool_vault_0.key(),
        VaultError::InvalidPoolVaults
    );
    require_keys_eq!(
        parsed_pool.token_vault_1,
        ctx.accounts.pool_vault_1.key(),
        VaultError::InvalidPoolVaults
    );

    let parsed_pos = parse_personal_position(
        &ctx.accounts.personal_position.to_account_info(),
        &ctx.accounts.clmm_program.key(),
    )?;

    let deposit_is_token_0 = parsed_pool.token_mint_0 == ctx.accounts.vault.deposit_mint;
    let (recipient_0, recipient_1, vault_0_mint, vault_1_mint) = ordered_vault_accounts(
        deposit_is_token_0,
        &ctx.accounts.token_vault,
        &ctx.accounts.secondary_vault,
        &ctx.accounts.deposit_mint,
        &ctx.accounts.secondary_mint,
    );

    let (before_0, before_1) = if deposit_is_token_0 {
        (
            ctx.accounts.token_vault.amount,
            ctx.accounts.secondary_vault.amount,
        )
    } else {
        (
            ctx.accounts.secondary_vault.amount,
            ctx.accounts.token_vault.amount,
        )
    };

    let deposit_mint_key = ctx.accounts.vault.deposit_mint;
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_SEED,
        deposit_mint_key.as_ref(),
        &[ctx.accounts.vault.bump],
    ]];

    if parsed_pos.liquidity > 0 || parsed_pos.token_fees_owed_0 > 0 || parsed_pos.token_fees_owed_1 > 0
    {
        invoke_decrease_liquidity_v2(
            &ctx.accounts.clmm_program.to_account_info(),
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.position_nft_account.to_account_info(),
            &ctx.accounts.personal_position.to_account_info(),
            &ctx.accounts.pool_state.to_account_info(),
            &ctx.accounts.protocol_position.to_account_info(),
            &ctx.accounts.pool_vault_0.to_account_info(),
            &ctx.accounts.pool_vault_1.to_account_info(),
            &ctx.accounts.tick_array_lower.to_account_info(),
            &ctx.accounts.tick_array_upper.to_account_info(),
            &recipient_0,
            &recipient_1,
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.token_program_2022.to_account_info(),
            &ctx.accounts.memo_program.to_account_info(),
            &vault_0_mint,
            &vault_1_mint,
            ctx.remaining_accounts,
            parsed_pos.liquidity,
            amount_0_min,
            amount_1_min,
            signer_seeds,
        )?;
    }

    ctx.accounts.token_vault.reload()?;
    ctx.accounts.secondary_vault.reload()?;

    let (after_0, after_1) = if deposit_is_token_0 {
        (
            ctx.accounts.token_vault.amount,
            ctx.accounts.secondary_vault.amount,
        )
    } else {
        (
            ctx.accounts.secondary_vault.amount,
            ctx.accounts.token_vault.amount,
        )
    };

    let received_0 = after_0.checked_sub(before_0).ok_or(VaultError::MathOverflow)?;
    let received_1 = after_1.checked_sub(before_1).ok_or(VaultError::MathOverflow)?;
    require!(
        received_0 >= amount_0_min && received_1 >= amount_1_min,
        VaultError::SlippageExceeded
    );

    let vault_lamports_before = ctx.accounts.vault.to_account_info().lamports();

    invoke_close_position(
        &ctx.accounts.clmm_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.position_nft_mint.to_account_info(),
        &ctx.accounts.position_nft_account.to_account_info(),
        &ctx.accounts.personal_position.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.token_program_2022.to_account_info(),
        signer_seeds,
    )?;

    // Возвращаем высвобожденный rent от закрытия NFT и PersonalPositionState обратно `admin`.
    let vault_info = ctx.accounts.vault.to_account_info();
    let vault_lamports_after = vault_info.lamports();
    if vault_lamports_after > vault_lamports_before {
        let reclaimed = vault_lamports_after - vault_lamports_before;
        **vault_info.try_borrow_mut_lamports()? -= reclaimed;
        **ctx
            .accounts
            .admin
            .to_account_info()
            .try_borrow_mut_lamports()? += reclaimed;
    }

    let strategy = &mut ctx.accounts.strategy;
    strategy.position_nft_mint = Pubkey::default();
    strategy.position_nft_account = Pubkey::default();
    strategy.personal_position = Pubkey::default();
    strategy.tick_lower_index = 0;
    strategy.tick_upper_index = 0;
    strategy.liquidity = 0;
    strategy.active_position = false;

    Ok(())
}
