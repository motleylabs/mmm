use anchor_lang::{
    prelude::*,
    solana_program::{program::invoke_signed, sysvar},
    AnchorDeserialize,
};
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};
use mpl_token_auth_rules::payload::{Payload, PayloadType, SeedsVec};
use mpl_token_metadata::{
    instruction::{builders::TransferBuilder, InstructionBuilder, TransferArgs},
    processor::AuthorizationData,
    state::{Metadata, TokenMetadataAccount},
};

use crate::{
    constants::*,
    errors::MMMErrorCode,
    instructions::vanilla::WithdrawSellArgs,
    state::{Pool, SellState},
    util::{assert_is_programmable, log_pool, try_close_pool, try_close_sell_state},
};

#[derive(Accounts)]
#[instruction(args:WithdrawSellArgs)]
pub struct Mip1WithdrawSell<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    pub cosigner: Signer<'info>,
    #[account(
        mut,
        seeds = [POOL_PREFIX.as_bytes(), owner.key().as_ref(), pool.uuid.as_ref()],
        has_one = owner @ MMMErrorCode::InvalidOwner,
        has_one = cosigner @ MMMErrorCode::InvalidCosigner,
        bump
    )]
    pub pool: Box<Account<'info, Pool>>,
    #[account(
        constraint = asset_mint.supply == 1 && asset_mint.decimals == 0 @ MMMErrorCode::InvalidMip1AssetParams,
    )]
    pub asset_mint: Account<'info, Mint>,
    /// CHECK: will be checked in cpi
    asset_master_edition: UncheckedAccount<'info>,
    /// CHECK: will be checked in cpi
    #[account(mut)]
    pub asset_metadata: UncheckedAccount<'info>,
    #[account(
        init_if_needed,
        associated_token::mint = asset_mint,
        associated_token::authority = owner,
        payer = owner
    )]
    pub asset_token_account: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = asset_mint,
        associated_token::authority = pool,
        constraint = sellside_escrow_token_account.amount == 1 @ MMMErrorCode::InvalidMip1AssetParams,
        constraint = args.asset_amount == 1 @ MMMErrorCode::InvalidMip1AssetParams,
    )]
    pub sellside_escrow_token_account: Box<Account<'info, TokenAccount>>,
    /// CHECK: it's a pda, and the private key is owned by the seeds
    #[account(
        mut,
        seeds = [BUYSIDE_SOL_ESCROW_ACCOUNT_PREFIX.as_bytes(), pool.key().as_ref()],
        bump,
    )]
    pub buyside_sol_escrow_account: UncheckedAccount<'info>,
    /// CHECK: will be used for allowlist checks
    pub allowlist_aux_account: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [
            SELL_STATE_PREFIX.as_bytes(),
            pool.key().as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump
    )]
    pub sell_state: Account<'info, SellState>,
    /// CHECK: will be checked in cpi
    #[account(mut)]
    pub owner_token_record: UncheckedAccount<'info>,
    /// CHECK: will be checked in cpi
    #[account(mut)]
    pub destination_token_record: UncheckedAccount<'info>,
    /// CHECK: will be checked in cpi
    pub authorization_rules: UncheckedAccount<'info>,

    /// CHECK: checked by address and in cpi
    #[account(address = mpl_token_metadata::id())]
    pub token_metadata_program: UncheckedAccount<'info>,
    /// CHECK: checked by address and in cpi
    #[account(address = mpl_token_auth_rules::id())]
    pub authorization_rules_program: UncheckedAccount<'info>,
    /// CHECK: checked by address and in cpi
    #[account(address = sysvar::instructions::id())]
    pub instructions: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handler(ctx: Context<Mip1WithdrawSell>, args: WithdrawSellArgs) -> Result<()> {
    let owner = &ctx.accounts.owner;
    let asset_token_account = &ctx.accounts.asset_token_account;
    let sellside_escrow_token_account = &ctx.accounts.sellside_escrow_token_account;
    let token_program = &ctx.accounts.token_program;
    let buyside_sol_escrow_account = &ctx.accounts.buyside_sol_escrow_account;
    let pool = &mut ctx.accounts.pool;
    let sell_state = &mut ctx.accounts.sell_state;
    let asset_mint = &ctx.accounts.asset_mint;
    let asset_metadata = &ctx.accounts.asset_metadata;
    let asset_master_edition = &ctx.accounts.asset_master_edition;
    let owner_token_record = &ctx.accounts.owner_token_record;
    let destination_token_record = &ctx.accounts.destination_token_record;
    let system_program = &ctx.accounts.system_program;
    let instructions = &ctx.accounts.instructions;
    let associated_token_program = &ctx.accounts.associated_token_program;
    let authorization_rules = &ctx.accounts.authorization_rules;
    let authorization_rules_program = &ctx.accounts.authorization_rules_program;

    let pool_seeds: &[&[&[u8]]] = &[&[
        POOL_PREFIX.as_bytes(),
        pool.owner.as_ref(),
        pool.uuid.as_ref(),
        &[*ctx.bumps.get("pool").unwrap()],
    ]];

    assert_is_programmable(&Metadata::from_account_info(asset_metadata)?)?;

    let payload = Payload::from([(
        "SourceSeeds".to_owned(),
        PayloadType::Seeds(SeedsVec {
            seeds: pool_seeds[0][0..3].iter().map(|v| v.to_vec()).collect(),
        }),
    )]);
    let ins = TransferBuilder::new()
        .token(sellside_escrow_token_account.key())
        .token_owner(pool.key())
        .destination(asset_token_account.key())
        .destination_owner(owner.key())
        .mint(asset_mint.key())
        .metadata(asset_metadata.key())
        .edition(asset_master_edition.key())
        .owner_token_record(owner_token_record.key())
        .destination_token_record(destination_token_record.key())
        .authority(pool.key())
        .payer(owner.key())
        .system_program(system_program.key())
        .sysvar_instructions(instructions.key())
        .spl_token_program(token_program.key())
        .spl_ata_program(associated_token_program.key())
        .authorization_rules(authorization_rules.key())
        .authorization_rules_program(authorization_rules_program.key())
        .build(TransferArgs::V1 {
            authorization_data: Some(AuthorizationData { payload }),
            amount: args.asset_amount,
        })
        .unwrap()
        .instruction();

    invoke_signed(
        &ins,
        &[
            sellside_escrow_token_account.to_account_info(),
            pool.to_account_info(),
            asset_token_account.to_account_info(),
            owner.to_account_info(),
            asset_mint.to_account_info(),
            asset_metadata.to_account_info(),
            asset_master_edition.to_account_info(),
            owner_token_record.to_account_info(),
            destination_token_record.to_account_info(),
            system_program.to_account_info(),
            instructions.to_account_info(),
            token_program.to_account_info(),
            associated_token_program.to_account_info(),
            authorization_rules.to_account_info(),
            authorization_rules_program.to_account_info(),
        ],
        pool_seeds,
    )?;

    if sellside_escrow_token_account.amount == args.asset_amount {
        anchor_spl::token::close_account(CpiContext::new_with_signer(
            token_program.to_account_info(),
            anchor_spl::token::CloseAccount {
                account: sellside_escrow_token_account.to_account_info(),
                destination: owner.to_account_info(),
                authority: pool.to_account_info(),
            },
            pool_seeds,
        ))?;
    }

    pool.sellside_asset_amount = pool
        .sellside_asset_amount
        .checked_sub(args.asset_amount)
        .ok_or(MMMErrorCode::NumericOverflow)?;
    sell_state.asset_amount = sell_state
        .asset_amount
        .checked_sub(args.asset_amount)
        .ok_or(MMMErrorCode::NumericOverflow)?;
    try_close_sell_state(sell_state, owner.to_account_info())?;

    pool.buyside_payment_amount = buyside_sol_escrow_account.lamports();
    log_pool("post_mip1_withdraw_sell", pool)?;
    try_close_pool(pool, owner.to_account_info())?;

    Ok(())
}
