import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { RiseCdp } from "../target/types/rise_cdp";
import { RiseStaking } from "../target/types/rise_staking";
import {
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
  Transaction,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  mintToChecked,
  getAccount,
  getOrCreateAssociatedTokenAccount,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountIdempotentInstruction,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { assert } from "chai";
import { buildCdpPriceUpdateIxs } from "./pyth-pull";

// Pyth pull-oracle feed IDs (32-byte hex, same on devnet and mainnet)
const USDC_FEED_ID_HEX = "eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a";
const SOL_FEED_ID_HEX  = "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d";
// Feed ID pubkeys stored in CollateralConfig / PaymentConfig on-chain
const usdcFeedId = new PublicKey(Buffer.from(USDC_FEED_ID_HEX, "hex"));
const solFeedId  = new PublicKey(Buffer.from(SOL_FEED_ID_HEX, "hex"));

const MIN_DEPLOYER_BALANCE = 2 * LAMPORTS_PER_SOL; // 2 SOL covers all devnet test transactions

describe("rise-cdp", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const cdpProgram = anchor.workspace.RiseCdp as Program<RiseCdp>;
  const stakingProgram = anchor.workspace.RiseStaking as Program<RiseStaking>;
  const authority = provider.wallet as anchor.Wallet;

  let usdcMint: PublicKey;
  let riseSolMint: PublicKey;
  let riseMint: PublicKey;             // RISE governance/rewards token
  let globalPool: PublicKey;
  let poolVault: PublicKey;
  let cdpConfig: PublicKey;
  let treasuryVault: PublicKey;
  let collateralConfig: PublicKey;
  let collateralVault: PublicKey;
  let position: PublicKey;
  let userUsdcAccount: PublicKey;
  let userRiseSolAccount: PublicKey;
  let userRiseAccount: PublicKey;      // borrower's RISE token account for claiming
  let borrowRewardsConfig: PublicKey;
  let borrowRewardsVault: PublicKey;

  const NONCE = 0;

  // Collateral config params for USDC (kinked interest rate model)
  const USDC_MAX_LTV_BPS = 8500;
  const USDC_LIQ_THRESHOLD_BPS = 9000;
  const USDC_LIQ_PENALTY_BPS = 500;
  const USDC_BASE_RATE_BPS = 300;       // 3% floor rate
  const USDC_RATE_SLOPE1_BPS = 400;     // +4% from 0% → 80% utilization
  const USDC_RATE_SLOPE2_BPS = 7500;    // +75% from 80% → 100% utilization
  const USDC_OPTIMAL_UTIL_BPS = 8000;   // 80% optimal utilization
  const USDC_SLIPPAGE_BPS = 50;

  before(async () => {
    const balance = await provider.connection.getBalance(authority.publicKey);
    assert.isTrue(
      balance >= MIN_DEPLOYER_BALANCE,
      `Deployer wallet needs ≥ 2 SOL for devnet tests, current balance: ${balance / LAMPORTS_PER_SOL} SOL`
    );

    // Derive staking PDAs
    [globalPool] = PublicKey.findProgramAddressSync(
      [Buffer.from("global_pool")],
      stakingProgram.programId
    );
    [poolVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("pool_vault")],
      stakingProgram.programId
    );

    // Create USDC mock mint (6 decimals). Use "confirmed" so the mint account is
    // visible to all RPC nodes before we create an ATA for it.
    usdcMint = await createMint(
      provider.connection,
      authority.payer,
      authority.publicKey,
      null,
      6,
      undefined,
      { commitment: "confirmed" }
    );

    // Use the existing GlobalPool's riseSOL mint if the pool is already initialized,
    // otherwise create a new mint with the pool PDA as authority.
    const poolInfo = await provider.connection.getAccountInfo(globalPool);
    if (poolInfo !== null) {
      const poolData = await stakingProgram.account.globalPool.fetch(globalPool);
      riseSolMint = poolData.riseSolMint;
      console.log("Reusing existing riseSOL mint:", riseSolMint.toBase58());
    } else {
      riseSolMint = await createMint(
        provider.connection,
        authority.payer,
        globalPool,
        null,
        9
      );
    }

    // Derive ATA address deterministically then create it idempotently.
    // Avoids the post-creation fetch race that causes TokenAccountNotFoundError on devnet.
    userUsdcAccount = getAssociatedTokenAddressSync(usdcMint, authority.publicKey);
    await provider.sendAndConfirm(
      new Transaction().add(
        createAssociatedTokenAccountIdempotentInstruction(
          authority.publicKey,
          userUsdcAccount,
          authority.publicKey,
          usdcMint
        )
      )
    );

    try {
      userRiseSolAccount = await createAccount(
        provider.connection,
        authority.payer,
        riseSolMint,
        authority.publicKey
      );
    } catch {
      const accounts = await provider.connection.getTokenAccountsByOwner(
        authority.publicKey, { mint: riseSolMint }
      );
      userRiseSolAccount = accounts.value[0].pubkey;
    }

    // Mint 10,000 USDC to user
    await mintTo(
      provider.connection,
      authority.payer,
      usdcMint,
      userUsdcAccount,
      authority.publicKey,
      10_000 * 1_000_000
    );

    // Derive PDAs
    [cdpConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("cdp_config")],
      cdpProgram.programId
    );

    [treasuryVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("treasury_vault")],
      stakingProgram.programId
    );

    [collateralConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("collateral_config"), usdcMint.toBuffer()],
      cdpProgram.programId
    );

    [collateralVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("collateral_vault"), usdcMint.toBuffer()],
      cdpProgram.programId
    );

    [position] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("cdp_position"),
        authority.publicKey.toBuffer(),
        Buffer.from([NONCE]),
      ],
      cdpProgram.programId
    );

    // userRiseAccount is created in the borrow_rewards describe block

    // Derive borrow rewards PDAs
    [borrowRewardsConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("borrow_rewards_config")],
      cdpProgram.programId
    );
    [borrowRewardsVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("borrow_rewards_vault")],
      cdpProgram.programId
    );

    // Reuse the RISE mint from the on-chain borrowRewardsConfig if it already exists,
    // otherwise create a fresh one. Keeps riseMint consistent across test runs.
    const brConfigInfo = await provider.connection.getAccountInfo(borrowRewardsConfig);
    if (brConfigInfo !== null) {
      const brConfig = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);
      riseMint = brConfig.riseMint;
      console.log("Reusing existing RISE mint from borrowRewardsConfig:", riseMint.toBase58());
    } else {
      riseMint = await createMint(
        provider.connection,
        authority.payer,
        authority.publicKey,
        null,
        6
      );
    }

    console.log("USDC mint:", usdcMint.toBase58());
    console.log("riseSOL mint:", riseSolMint.toBase58());
    console.log("RISE mint:", riseMint.toBase58());
    console.log("Collateral config PDA:", collateralConfig.toBase58());
    console.log("Position PDA:", position.toBase58());
  });

  it("Initializes the staking pool", async () => {
    const poolInfo = await provider.connection.getAccountInfo(globalPool);
    if (poolInfo !== null) {
      console.log("Pool already initialized — skipping");
      return;
    }
    await stakingProgram.methods
      .initializePool(1000, 500)
      .accounts({
        authority: authority.publicKey,
        pool: globalPool,
        riseSolMint: riseSolMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
    console.log("Staking pool initialized");
  });

  it("Initializes the CDP config", async () => {
    const cdpConfigInfo = await provider.connection.getAccountInfo(cdpConfig);
    if (cdpConfigInfo !== null) {
      console.log("CDP config already initialized — skipping");
      return;
    }
    // 30000 bps = 3x staking supply debt ceiling
    await cdpProgram.methods
      .initializeCdpConfig(30000)
      .accounts({
        authority: authority.publicKey,
        cdpConfig,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    console.log("CDP config initialized");
  });

  it("Initializes borrow rewards config", async () => {
    const existing = await provider.connection.getAccountInfo(borrowRewardsConfig);
    if (existing) {
      console.log("Borrow rewards config already initialized — skipping");
      return;
    }

    // 1_000_000 RISE per epoch (6 decimals), ~1 week in slots
    const epochEmissions = new anchor.BN(1_000_000 * 1_000_000);
    const slotsPerEpoch = new anchor.BN(604_800);

    await cdpProgram.methods
      .initializeBorrowRewards(epochEmissions, slotsPerEpoch)
      .accounts({
        authority: authority.publicKey,
        cdpConfig,
        borrowRewardsConfig,
        rewardsVault: borrowRewardsVault,
        riseMint,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const cfg = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);
    assert.equal(cfg.riseMint.toBase58(), riseMint.toBase58());
    assert.equal(cfg.epochEmissions.toNumber(), epochEmissions.toNumber());
    assert.equal(cfg.totalCdpDebt.toNumber(), 0);
    assert.equal(cfg.rewardPerToken.toString(), "0");

    console.log("Borrow rewards config initialized");
    console.log("Epoch emissions:", cfg.epochEmissions.toString(), "RISE");
  });

  it("Initializes USDC collateral config", async () => {
    await cdpProgram.methods
      .initializeCollateralConfig(
        usdcFeedId,
        USDC_MAX_LTV_BPS,
        USDC_LIQ_THRESHOLD_BPS,
        USDC_LIQ_PENALTY_BPS,
        USDC_BASE_RATE_BPS,
        USDC_RATE_SLOPE1_BPS,
        USDC_RATE_SLOPE2_BPS,
        USDC_OPTIMAL_UTIL_BPS,
        USDC_SLIPPAGE_BPS
      )
      .accounts({
        authority: authority.publicKey,
        collateralConfig: collateralConfig,
        collateralMint: usdcMint,
        systemProgram: SystemProgram.programId,
      })
      .rpc({ commitment: "confirmed" });

    const config = await cdpProgram.account.collateralConfig.fetch(collateralConfig, "confirmed");

    assert.equal(config.mint.toBase58(), usdcMint.toBase58());
    assert.equal(config.maxLtvBps, USDC_MAX_LTV_BPS);
    assert.equal(config.liquidationThresholdBps, USDC_LIQ_THRESHOLD_BPS);
    assert.equal(config.liquidationPenaltyBps, USDC_LIQ_PENALTY_BPS);
    assert.equal(config.baseRateBps, USDC_BASE_RATE_BPS);
    assert.equal(config.rateSlope1Bps, USDC_RATE_SLOPE1_BPS);
    assert.equal(config.rateSlope2Bps, USDC_RATE_SLOPE2_BPS);
    assert.equal(config.optimalUtilizationBps, USDC_OPTIMAL_UTIL_BPS);
    assert.equal(config.active, true);

    console.log("USDC collateral config initialized");
    console.log("Max LTV:", config.maxLtvBps, "bps");
    console.log("Base rate:", config.baseRateBps, "bps | Slope1:", config.rateSlope1Bps, "| Slope2:", config.rateSlope2Bps);
  });

  it("Updates collateral config base rate", async () => {
    const newRate = 400; // 4%

    await cdpProgram.methods
      .updateCollateralConfig(
        null,     // feed_id — no change
        null,     // max_ltv_bps — no change
        null,     // liquidation_threshold_bps — no change
        null,     // liquidation_penalty_bps — no change
        newRate,  // base_rate_bps — update this
        null,     // rate_slope1_bps — no change
        null,     // rate_slope2_bps — no change
        null,     // optimal_utilization_bps — no change
        null,     // conversion_slippage_bps — no change
        null      // active — no change
      )
      .accounts({
        authority: authority.publicKey,
        collateralConfig: collateralConfig,
      })
      .rpc({ commitment: "confirmed" });

    const config = await cdpProgram.account.collateralConfig.fetch(collateralConfig, "confirmed");
    assert.equal(config.baseRateBps, newRate);
    console.log("Base rate updated to:", config.baseRateBps, "bps");

    // Reset back to original rate
    await cdpProgram.methods
      .updateCollateralConfig(null, null, null, null, USDC_BASE_RATE_BPS, null, null, null, null, null)
      .accounts({
        authority: authority.publicKey,
        collateralConfig: collateralConfig,
      })
      .rpc({ commitment: "confirmed" });

    const configReset = await cdpProgram.account.collateralConfig.fetch(collateralConfig, "confirmed");
    assert.equal(configReset.baseRateBps, USDC_BASE_RATE_BPS);
    console.log("Base rate reset to:", USDC_BASE_RATE_BPS, "bps");
  });

  it("Deactivates and reactivates a collateral type", async () => {
    // If a previous run left collateral deactivated, reactivate it first so we
    // start from a known active state.
    const startState = await cdpProgram.account.collateralConfig.fetch(collateralConfig, "confirmed");
    if (!startState.active) {
      await cdpProgram.methods
        .updateCollateralConfig(null, null, null, null, null, null, null, null, null, true)
        .accounts({
          authority: authority.publicKey,
          collateralConfig: collateralConfig,
        })
        .rpc({ commitment: "confirmed" });
      console.log("Collateral was already deactivated — reactivated to reset state");
    }

    // Deactivate
    await cdpProgram.methods
      .updateCollateralConfig(null, null, null, null, null, null, null, null, null, false)
      .accounts({
        authority: authority.publicKey,
        collateralConfig: collateralConfig,
      })
      .rpc({ commitment: "confirmed" });

    let config = await cdpProgram.account.collateralConfig.fetch(collateralConfig, "confirmed");
    assert.equal(config.active, false);
    console.log("Collateral deactivated");

    // Reactivate
    await cdpProgram.methods
      .updateCollateralConfig(null, null, null, null, null, null, null, null, null, true)
      .accounts({
        authority: authority.publicKey,
        collateralConfig: collateralConfig,
      })
      .rpc({ commitment: "confirmed" });

    config = await cdpProgram.account.collateralConfig.fetch(collateralConfig, "confirmed");
    assert.equal(config.active, true);
    console.log("Collateral reactivated");
  });

  it("CDP program has all expected instructions", async () => {
    const idl = cdpProgram.idl;
    const instructionNames = idl.instructions.map((ix: any) => ix.name);

    assert.include(instructionNames, "initializeCollateralConfig");
    assert.include(instructionNames, "updateCollateralConfig");
    assert.include(instructionNames, "openPosition");
    assert.include(instructionNames, "closePosition");
    assert.include(instructionNames, "addCollateral");
    assert.include(instructionNames, "withdrawExcess");
    assert.include(instructionNames, "liquidate");
    assert.include(instructionNames, "accrueInterest");
    assert.include(instructionNames, "initializePaymentConfig");
    assert.include(instructionNames, "repayDebt");
    assert.include(instructionNames, "borrowMore");
    assert.include(instructionNames, "collectCdpFees");
    assert.include(instructionNames, "repayDebtRiseSol");

    console.log("All CDP instructions present:", instructionNames);
  });

  // ── New instruction tests ────────────────────────────────────────────────

  describe("initialize_payment_config", () => {
    it("Initializes a SOL payment config", async () => {
      const solMintSentinel = anchor.web3.SystemProgram.programId;

      const [paymentConfig] = PublicKey.findProgramAddressSync(
        [Buffer.from("payment_config"), solMintSentinel.toBuffer()],
        cdpProgram.programId
      );

      // Skip if already exists (idempotent across test runs)
      const existing = await provider.connection.getAccountInfo(paymentConfig);
      if (existing) {
        console.log("SOL payment config already exists — skipping");
        return;
      }

      await cdpProgram.methods
        .initializePaymentConfig(solFeedId)
        .accounts({
          authority: authority.publicKey,
          paymentConfig,
          mint: solMintSentinel,
          systemProgram: SystemProgram.programId,
        })
        .rpc({ commitment: "confirmed" });

      const cfg = await cdpProgram.account.paymentConfig.fetch(paymentConfig, "confirmed");
      assert.equal(cfg.mint.toBase58(), solMintSentinel.toBase58());
      assert.equal(cfg.pythPriceFeed.toBase58(), solFeedId.toBase58());
      assert.equal(cfg.active, true);

      console.log("SOL payment config initialized");
      console.log("Mint (sentinel):", cfg.mint.toBase58());
    });

    it("Initializes a USDC payment config", async () => {
      const [paymentConfig] = PublicKey.findProgramAddressSync(
        [Buffer.from("payment_config"), usdcMint.toBuffer()],
        cdpProgram.programId
      );

      const existing = await provider.connection.getAccountInfo(paymentConfig);
      if (existing) {
        console.log("USDC payment config already exists — skipping");
        return;
      }

      await cdpProgram.methods
        .initializePaymentConfig(usdcFeedId)
        .accounts({
          authority: authority.publicKey,
          paymentConfig,
          mint: usdcMint,
          systemProgram: SystemProgram.programId,
        })
        .rpc({ commitment: "confirmed" });

      const cfg = await cdpProgram.account.paymentConfig.fetch(paymentConfig, "confirmed");
      assert.equal(cfg.mint.toBase58(), usdcMint.toBase58());
      assert.equal(cfg.active, true);

      console.log("USDC payment config initialized");
    });
  });

  describe("repay_debt + borrow_more", () => {
    // These tests open a position, borrow more, then partially and fully repay.

    let cdpFeeVault: PublicKey;
    let poolVault: PublicKey;
    let stakingTreasury: PublicKey;
    let solPaymentConfig: PublicKey;

    let borrowRewards: PublicKey;

    before(async () => {
      [cdpFeeVault] = PublicKey.findProgramAddressSync(
        [Buffer.from("cdp_fee_vault")],
        cdpProgram.programId
      );
      [poolVault] = PublicKey.findProgramAddressSync(
        [Buffer.from("pool_vault")],
        stakingProgram.programId
      );
      [stakingTreasury] = PublicKey.findProgramAddressSync(
        [Buffer.from("protocol_treasury")],
        stakingProgram.programId
      );
      [solPaymentConfig] = PublicKey.findProgramAddressSync(
        [Buffer.from("payment_config"), anchor.web3.SystemProgram.programId.toBuffer()],
        cdpProgram.programId
      );

      // borrow_rewards is seeded from the position PDA (derived at top-level before)
      [borrowRewards] = PublicKey.findProgramAddressSync(
        [Buffer.from("borrow_rewards"), position.toBuffer()],
        cdpProgram.programId
      );
    });

    it("Initializes the USDC collateral vault", async () => {
      const vaultInfo = await provider.connection.getAccountInfo(collateralVault);
      if (vaultInfo) {
        console.log("Collateral vault already exists — skipping");
        return;
      }
      await cdpProgram.methods
        .initializeCollateralVault()
        .accounts({
          authority: authority.publicKey,
          collateralConfig,
          collateralMint: usdcMint,
          collateralVault,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc({ commitment: "confirmed" });
      console.log("Collateral vault initialized:", collateralVault.toBase58());
    });

    it("Opens a CDP position (collateral in, debt recorded)", async () => {
      // Check if pool is initialized; init if needed
      const poolInfo = await provider.connection.getAccountInfo(globalPool);
      if (!poolInfo) {
        await stakingProgram.methods
          .initializePool(1000, 500)
          .accounts({
            authority: authority.publicKey,
            pool: globalPool,
            riseSolMint,
            systemProgram: SystemProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc();
      }

      // Stake SOL to establish staking_rise_sol_supply > 0 so the debt ceiling is non-zero.
      // With debt_ceiling_multiplier=3x and MAX_SINGLE_LOAN_BPS=500 (5%):
      //   single_loan_cap = supply * 3 * 0.05 = supply * 0.15
      //   To borrow 5 SOL: supply >= 5 / 0.15 ≈ 33.3 SOL → stake 40 SOL on localnet.
      //   On devnet where the wallet has limited SOL, set BOOTSTRAP_SOL=5 in the environment
      //   (devnet state persists across runs so the supply accumulates over time).
      const bootstrapSol = process.env.BOOTSTRAP_SOL ? parseInt(process.env.BOOTSTRAP_SOL) : 40;
      const bootstrapLamports = bootstrapSol * LAMPORTS_PER_SOL;
      const poolData = await stakingProgram.account.globalPool.fetch(globalPool);
      const currentStake = poolData.totalSolStaked.toNumber();
      if (currentStake < bootstrapLamports) {
        const topUp = bootstrapLamports - currentStake;
        await stakingProgram.methods
          .stakeSol(new anchor.BN(topUp))
          .accounts({
            user: authority.publicKey,
            pool: globalPool,
            poolVault,
            riseSolMint,
            userRiseSolAccount,
            systemProgram: SystemProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
            stakeRewardsConfig: null,
            userStakeRewards: null,
          })
          .rpc();
        console.log(
          `Staked ${topUp / LAMPORTS_PER_SOL} SOL to reach ${bootstrapSol} SOL bootstrap target`
        );
      } else {
        console.log(
          `Pool already has ${currentStake / LAMPORTS_PER_SOL} SOL staked — skipping bootstrap`
        );
      }

      // Register cdpConfig on the staking program so mint_for_cdp CPI is authorized.
      const freshPool = await stakingProgram.account.globalPool.fetch(globalPool);
      if (freshPool.cdpConfigPubkey.equals(PublicKey.default)) {
        await stakingProgram.methods
          .setCdpConfig(cdpConfig)
          .accounts({
            authority: authority.publicKey,
            globalPool,
          })
          .rpc();
        console.log("CDP config registered on staking program");
      }

      const positionInfo = await provider.connection.getAccountInfo(position);
      if (positionInfo) {
        console.log("Position already exists — skipping open");
        return;
      }

      // Deposit 1000 USDC, borrow riseSOL worth ~$750 at $150 SOL
      // max_borrow_lamports = 1_000_000 USD * 1e9 / 150_000_000 * 8500 / 10000
      //   = ~5_666_666_666 lamports → use 5_000_000_000 to stay safe
      const collateralAmount = 1_000 * 1_000_000; // 1000 USDC (6 dec)
      const riseSolToBorrow = 5_000_000_000; // 5 riseSOL in lamports (9 dec)

      const { priceUpdateKeypair: puKp0, solPriceUpdateKeypair: spuKp0, priceUpdateIx: puIx0, solPriceUpdateIx: spuIx0 } =
        await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

      await cdpProgram.methods
        .openPosition(
          new anchor.BN(collateralAmount),
          new anchor.BN(riseSolToBorrow),
          NONCE
        )
        .accounts({
          borrower: authority.publicKey,
          cdpConfig,
          globalPool,
          position,
          collateralConfig,
          collateralMint: usdcMint,
          borrowerCollateralAccount: userUsdcAccount,
          collateralVault,
          priceUpdate: puKp0.publicKey,
          solPriceUpdate: spuKp0.publicKey,
          riseSolMint,
          borrowerRiseSolAccount: userRiseSolAccount,
          stakingProgram: stakingProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          borrowRewardsConfig,
          borrowRewards,
        })
        .preInstructions([puIx0, spuIx0])
        .signers([puKp0, spuKp0])
        .rpc();

      const pos = await cdpProgram.account.cdpPosition.fetch(position);
      assert.equal(pos.isOpen, true);
      assert.equal(pos.riseSolDebtPrincipal.toNumber(), riseSolToBorrow);
      assert.equal(pos.interestAccrued.toNumber(), 0);

      console.log("Position opened — principal:", pos.riseSolDebtPrincipal.toString());
      console.log("Health factor:", pos.healthFactor.toString());
    });

    it("Borrows more riseSOL against the open position", async () => {
      const posBefore = await cdpProgram.account.cdpPosition.fetch(position);
      const additionalHsol = 500_000_000; // 0.5 riseSOL

      const { priceUpdateKeypair: puKp1, solPriceUpdateKeypair: spuKp1, priceUpdateIx: puIx1, solPriceUpdateIx: spuIx1 } =
        await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

      await cdpProgram.methods
        .borrowMore(new anchor.BN(additionalHsol))
        .accounts({
          borrower: authority.publicKey,
          position,
          collateralConfig,
          globalPool,
          cdpConfig,
          priceUpdate: puKp1.publicKey,
          solPriceUpdate: spuKp1.publicKey,
          riseSolMint,
          borrowerRiseSolAccount: userRiseSolAccount,
          stakingProgram: stakingProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          borrowRewardsConfig,
          borrowRewards,
        })
        .preInstructions([puIx1, spuIx1])
        .signers([puKp1, spuKp1])
        .rpc();

      const posAfter = await cdpProgram.account.cdpPosition.fetch(position);
      assert.equal(
        posAfter.riseSolDebtPrincipal.toNumber(),
        posBefore.riseSolDebtPrincipal.toNumber() + additionalHsol
      );

      console.log(
        "borrow_more: principal",
        posBefore.riseSolDebtPrincipal.toString(),
        "→",
        posAfter.riseSolDebtPrincipal.toString()
      );
    });

    it("Partially repays debt with native SOL", async () => {
      // Ensure SOL payment config exists
      const cfgInfo = await provider.connection.getAccountInfo(solPaymentConfig);
      if (!cfgInfo) {
        await cdpProgram.methods
          .initializePaymentConfig(solFeedId)
          .accounts({
            authority: authority.publicKey,
            paymentConfig: solPaymentConfig,
            mint: anchor.web3.SystemProgram.programId,
            systemProgram: SystemProgram.programId,
          })
          .rpc();
      }

      const posBefore = await cdpProgram.account.cdpPosition.fetch(position);
      const feeVaultBefore = await provider.connection.getBalance(cdpFeeVault);
      const poolVaultBefore = await provider.connection.getBalance(poolVault);

      // Pay 1 SOL (covers some principal; interest_accrued is 0 so all goes to principal)
      const paymentLamports = 1 * LAMPORTS_PER_SOL;

      const { priceUpdateKeypair: puKp2, solPriceUpdateKeypair: spuKp2, priceUpdateIx: puIx2, solPriceUpdateIx: spuIx2 } =
        await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

      await cdpProgram.methods
        .repayDebt(new anchor.BN(paymentLamports))
        .accounts({
          borrower: authority.publicKey,
          position,
          collateralConfig,
          paymentConfig: solPaymentConfig,
          globalPool,
          cdpConfig,
          cdpFeeVault,
          poolVault,
          collateralVault,
          borrowerCollateralAccount: userUsdcAccount,
          priceUpdate: puKp2.publicKey,
          solPriceUpdate: spuKp2.publicKey,
          paymentMint: null,
          borrowerPaymentAccount: null,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          borrowRewardsConfig,
          borrowRewards,
        })
        .preInstructions([puIx2, spuIx2])
        .signers([puKp2, spuKp2])
        .rpc();

      const posAfter = await cdpProgram.account.cdpPosition.fetch(position);

      // Position should still be open (partial repayment)
      assert.equal(posAfter.isOpen, true);
      // Principal should have decreased
      assert.isTrue(
        posAfter.riseSolDebtPrincipal.toNumber() < posBefore.riseSolDebtPrincipal.toNumber(),
        "Principal should decrease after partial repayment"
      );

      const feeVaultAfter = await provider.connection.getBalance(cdpFeeVault);
      const poolVaultAfter = await provider.connection.getBalance(poolVault);

      console.log(
        "Principal before:", posBefore.riseSolDebtPrincipal.toString(),
        "→ after:", posAfter.riseSolDebtPrincipal.toString()
      );
      console.log("Fee vault gained:", feeVaultAfter - feeVaultBefore, "lamports");
      console.log("Pool vault gained:", poolVaultAfter - poolVaultBefore, "lamports");
    });

    it("Fully repays remaining debt and closes position", async () => {
      const posBefore = await cdpProgram.account.cdpPosition.fetch(position);

      // Pay enough to cover remaining debt:
      // principal_remaining_rise_sol * exchange_rate / RATE_SCALE
      // exchange_rate starts at RATE_SCALE (1:1) since no rewards yet
      const pool = await stakingProgram.account.globalPool.fetch(globalPool);
      const exchangeRate = pool.exchangeRate.toNumber();
      const rateScale = 1_000_000_000;

      const remainingHsol =
        posBefore.riseSolDebtPrincipal.toNumber() +
        posBefore.interestAccrued.toNumber();
      const remainingSol = Math.ceil(
        (remainingHsol * exchangeRate) / rateScale
      );

      // Add a small buffer to ensure it fully clears (rounding)
      const paymentLamports = remainingSol + 1000;

      const collateralBefore = await getAccount(
        provider.connection,
        userUsdcAccount
      );

      const { priceUpdateKeypair: puKp3, solPriceUpdateKeypair: spuKp3, priceUpdateIx: puIx3, solPriceUpdateIx: spuIx3 } =
        await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

      await cdpProgram.methods
        .repayDebt(new anchor.BN(paymentLamports))
        .accounts({
          borrower: authority.publicKey,
          position,
          collateralConfig,
          paymentConfig: solPaymentConfig,
          globalPool,
          cdpConfig,
          cdpFeeVault,
          poolVault,
          collateralVault,
          borrowerCollateralAccount: userUsdcAccount,
          priceUpdate: puKp3.publicKey,
          solPriceUpdate: spuKp3.publicKey,
          paymentMint: null,
          borrowerPaymentAccount: null,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          borrowRewardsConfig,
          borrowRewards,
        })
        .preInstructions([puIx3, spuIx3])
        .signers([puKp3, spuKp3])
        .rpc();

      const posAfter = await cdpProgram.account.cdpPosition.fetch(position);
      assert.equal(posAfter.isOpen, false, "Position should be closed");
      assert.equal(posAfter.riseSolDebtPrincipal.toNumber(), 0);
      assert.equal(posAfter.interestAccrued.toNumber(), 0);

      const collateralAfter = await getAccount(
        provider.connection,
        userUsdcAccount
      );
      assert.isTrue(
        Number(collateralAfter.amount) > Number(collateralBefore.amount),
        "Collateral should be returned to borrower"
      );

      console.log("Position fully closed");
      console.log(
        "Collateral returned:",
        Number(collateralAfter.amount) - Number(collateralBefore.amount),
        "USDC base units"
      );
    });
  });

  describe("repay_debt_rise_sol", () => {
    const NONCE_RISE_SOL = 1; // separate position to avoid state collision

    it("Opens a second position and repays with riseSOL (partial then full)", async () => {
      // Derive a fresh position PDA with nonce 1
      const [position1] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("cdp_position"),
          authority.publicKey.toBuffer(),
          Buffer.from([NONCE_RISE_SOL]),
        ],
        cdpProgram.programId
      );

      // borrow_rewards for this position
      const [borrowRewards1] = PublicKey.findProgramAddressSync(
        [Buffer.from("borrow_rewards"), position1.toBuffer()],
        cdpProgram.programId
      );

      // Open the position: 500 USDC collateral, borrow 2 riseSOL
      const collateralAmount = 500 * 1_000_000;
      const riseSolToBorrow = 2_000_000_000;

      const posInfo = await provider.connection.getAccountInfo(position1);
      if (!posInfo) {
        const { priceUpdateKeypair: puKp4, solPriceUpdateKeypair: spuKp4, priceUpdateIx: puIx4, solPriceUpdateIx: spuIx4 } =
          await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

        await cdpProgram.methods
          .openPosition(
            new anchor.BN(collateralAmount),
            new anchor.BN(riseSolToBorrow),
            NONCE_RISE_SOL
          )
          .accounts({
            borrower: authority.publicKey,
            cdpConfig,
            globalPool,
            position: position1,
            collateralConfig,
            collateralMint: usdcMint,
            borrowerCollateralAccount: userUsdcAccount,
            borrowRewardsConfig,
            borrowRewards: borrowRewards1,
            collateralVault,
            priceUpdate: puKp4.publicKey,
            solPriceUpdate: spuKp4.publicKey,
            riseSolMint,
            borrowerRiseSolAccount: userRiseSolAccount,
            stakingProgram: stakingProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .preInstructions([puIx4, spuIx4])
          .signers([puKp4, spuKp4])
          .rpc();
      }

      // Mint some riseSOL directly to userRiseSolAccount for testing
      // (in production riseSOL would have been received at open_position time)
      await stakingProgram.methods
        .stakeSol(new anchor.BN(5 * LAMPORTS_PER_SOL))
        .accounts({
          user: authority.publicKey,
          pool: globalPool,
          poolVault: (PublicKey.findProgramAddressSync(
            [Buffer.from("pool_vault")],
            stakingProgram.programId
          ))[0],
          riseSolMint,
          userRiseSolAccount: userRiseSolAccount,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      const riseSolBalanceBefore = (await getAccount(provider.connection, userRiseSolAccount)).amount;

      // Partial repayment: burn 1 riseSOL
      const partialPayment = 1_000_000_000;
      await cdpProgram.methods
        .repayDebtRiseSol(new anchor.BN(partialPayment), [], new anchor.BN(0), 0)
        .accounts({
          borrower: authority.publicKey,
          position: position1,
          collateralConfig,
          riseSolMint,
          borrowerRiseSolAccount: userRiseSolAccount,
          collateralVault,
          borrowerCollateralAccount: userUsdcAccount,
          cdpConfig,
          globalPool,
          stakingProgram: stakingProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          borrowRewardsConfig,
          borrowRewards: borrowRewards1,
        })
        .rpc();

      let pos = await cdpProgram.account.cdpPosition.fetch(position1);
      assert.equal(pos.isOpen, true, "Position should still be open after partial repay");
      assert.equal(
        pos.riseSolDebtPrincipal.toNumber(),
        riseSolToBorrow - partialPayment,
        "Principal should decrease by exactly the riseSOL paid"
      );

      const riseSolAfterPartial = (await getAccount(provider.connection, userRiseSolAccount)).amount;
      assert.equal(
        Number(riseSolBalanceBefore) - Number(riseSolAfterPartial),
        partialPayment,
        "Borrower should have burned exactly the payment amount"
      );
      console.log("Partial riseSOL repay: principal reduced by", partialPayment, "riseSOL lamports");

      // Full repayment: burn remaining 1 riseSOL
      const collateralBefore = (await getAccount(provider.connection, userUsdcAccount)).amount;

      await cdpProgram.methods
        .repayDebtRiseSol(new anchor.BN(pos.riseSolDebtPrincipal.toNumber()), [], new anchor.BN(0), 0)
        .accounts({
          borrower: authority.publicKey,
          position: position1,
          collateralConfig,
          riseSolMint,
          borrowerRiseSolAccount: userRiseSolAccount,
          collateralVault,
          borrowerCollateralAccount: userUsdcAccount,
          cdpConfig,
          globalPool,
          stakingProgram: stakingProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          borrowRewardsConfig,
          borrowRewards: borrowRewards1,
        })
        .rpc();

      pos = await cdpProgram.account.cdpPosition.fetch(position1);
      assert.equal(pos.isOpen, false, "Position should be closed after full repay");
      assert.equal(pos.riseSolDebtPrincipal.toNumber(), 0);

      const collateralAfter = (await getAccount(provider.connection, userUsdcAccount)).amount;
      assert.isTrue(
        collateralAfter > collateralBefore,
        "Collateral should be returned after full riseSOL repayment"
      );
      console.log(
        "Full riseSOL repay: position closed, collateral returned:",
        Number(collateralAfter - collateralBefore),
        "USDC base units"
      );
    });
  });

  describe("borrow_rewards", () => {
    // Uses a fresh position with nonce 2 so it doesn't collide with earlier tests.
    const NONCE_BR = 2;
    let brPosition: PublicKey;
    let brBorrowRewards: PublicKey;
    let brUserRiseAccount: PublicKey;

    before(async () => {
      [brPosition] = PublicKey.findProgramAddressSync(
        [Buffer.from("cdp_position"), authority.publicKey.toBuffer(), Buffer.from([NONCE_BR])],
        cdpProgram.programId
      );
      [brBorrowRewards] = PublicKey.findProgramAddressSync(
        [Buffer.from("borrow_rewards"), brPosition.toBuffer()],
        cdpProgram.programId
      );

      // Create (or reuse) an associated token account for the borrower to receive RISE
      const riseAta = await getOrCreateAssociatedTokenAccount(
        provider.connection,
        authority.payer,
        riseMint,
        authority.publicKey
      );
      brUserRiseAccount = riseAta.address;
      userRiseAccount = brUserRiseAccount; // expose to outer scope for claim test

      // Ensure collateral is active — the "Deactivates and reactivates" test can leave it
      // deactivated if its assertion fails before the reactivation step runs.
      const collateralState = await cdpProgram.account.collateralConfig.fetch(collateralConfig);
      if (!collateralState.active) {
        await cdpProgram.methods
          .updateCollateralConfig(null, null, null, null, null, null, null, null, null, true)
          .accounts({
            authority: authority.publicKey,
            collateralConfig,
          })
          .rpc();
        console.log("borrow_rewards: reactivated collateral after earlier test left it deactivated");
      }
    });

    it("Initialize borrow rewards config stores correct state", async () => {
      const cfg = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);
      assert.equal(cfg.riseMint.toBase58(), riseMint.toBase58());
      assert.equal(cfg.rewardsVault.toBase58(), borrowRewardsVault.toBase58());
      assert.equal(cfg.rewardPerToken.toString(), "0");
      assert.equal(cfg.totalCdpDebt.toNumber(), 0);
      console.log("BorrowRewardsConfig confirmed on-chain");
    });

    it("Open position creates BorrowRewards with correct reward_debt", async () => {
      // Ensure the collateral vault is initialised (may have been skipped if already exists)
      const vaultInfo = await provider.connection.getAccountInfo(collateralVault);
      if (!vaultInfo) {
        await cdpProgram.methods
          .initializeCollateralVault()
          .accounts({
            authority: authority.publicKey,
            collateralConfig,
            collateralMint: usdcMint,
            collateralVault,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .rpc();
      }

      const collateralAmount = 500 * 1_000_000; // 500 USDC
      const riseSolToBorrow = 2_000_000_000;    // 2 riseSOL

      // Ensure this position doesn't already exist
      const posInfo = await provider.connection.getAccountInfo(brPosition);
      if (posInfo) {
        console.log("brPosition already exists — skipping open");
      } else {
        const { priceUpdateKeypair: puKp7, solPriceUpdateKeypair: spuKp7, priceUpdateIx: puIx7, solPriceUpdateIx: spuIx7 } =
          await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

        await cdpProgram.methods
          .openPosition(
            new anchor.BN(collateralAmount),
            new anchor.BN(riseSolToBorrow),
            NONCE_BR
          )
          .accounts({
            borrower: authority.publicKey,
            cdpConfig,
            globalPool,
            position: brPosition,
            collateralConfig,
            collateralMint: usdcMint,
            borrowerCollateralAccount: userUsdcAccount,
            collateralVault,
            priceUpdate: puKp7.publicKey,
            solPriceUpdate: spuKp7.publicKey,
            riseSolMint,
            borrowerRiseSolAccount: userRiseSolAccount,
            stakingProgram: stakingProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            borrowRewardsConfig,
            borrowRewards: brBorrowRewards,
          })
          .preInstructions([puIx7, spuIx7])
          .signers([puKp7, spuKp7])
          .rpc();
      }

      const cfg = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);
      const br = await cdpProgram.account.borrowRewards.fetch(brBorrowRewards);

      // reward_per_token is 0 at init, so reward_debt should be 0
      assert.equal(br.owner.toBase58(), authority.publicKey.toBase58());
      assert.equal(br.position.toBase58(), brPosition.toBase58());
      assert.equal(br.pendingRewards.toNumber(), 0);
      assert.equal(br.totalClaimed.toNumber(), 0);
      // reward_debt = debt * 0 / SCALE = 0
      assert.equal(br.rewardDebt.toString(), "0");

      // totalCdpDebt should include this position's borrow
      assert.isTrue(
        cfg.totalCdpDebt.toNumber() >= riseSolToBorrow,
        "totalCdpDebt should be at least the amount borrowed"
      );

      console.log("BorrowRewards initialized with reward_debt:", br.rewardDebt.toString());
      console.log("totalCdpDebt:", cfg.totalCdpDebt.toString());
    });

    it("Checkpoint updates reward_per_token correctly", async () => {
      const cfgBefore = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);

      await cdpProgram.methods
        .checkpointBorrowRewards()
        .accounts({
          caller: authority.publicKey,
          borrowRewardsConfig,
        })
        .rpc();

      const cfgAfter = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);

      // After a checkpoint with totalCdpDebt > 0, reward_per_token should be >= before
      assert.isTrue(
        cfgAfter.rewardPerToken.gte(cfgBefore.rewardPerToken),
        "reward_per_token should be non-decreasing"
      );
      assert.isTrue(
        cfgAfter.lastCheckpointSlot.toNumber() >= cfgBefore.lastCheckpointSlot.toNumber(),
        "lastCheckpointSlot should advance"
      );

      console.log("reward_per_token before:", cfgBefore.rewardPerToken.toString());
      console.log("reward_per_token after: ", cfgAfter.rewardPerToken.toString());
    });

    it("Borrow more correctly settles and updates reward_debt", async () => {
      const brBefore = await cdpProgram.account.borrowRewards.fetch(brBorrowRewards);
      const posBefore = await cdpProgram.account.cdpPosition.fetch(brPosition);
      const cfgBefore = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);

      const additionalBorrow = 500_000_000; // 0.5 riseSOL

      const { priceUpdateKeypair: puKp8, solPriceUpdateKeypair: spuKp8, priceUpdateIx: puIx8, solPriceUpdateIx: spuIx8 } =
        await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

      await cdpProgram.methods
        .borrowMore(new anchor.BN(additionalBorrow))
        .accounts({
          borrower: authority.publicKey,
          position: brPosition,
          collateralConfig,
          globalPool,
          cdpConfig,
          priceUpdate: puKp8.publicKey,
          solPriceUpdate: spuKp8.publicKey,
          riseSolMint,
          borrowerRiseSolAccount: userRiseSolAccount,
          stakingProgram: stakingProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          borrowRewardsConfig,
          borrowRewards: brBorrowRewards,
        })
        .preInstructions([puIx8, spuIx8])
        .signers([puKp8, spuKp8])
        .rpc();

      const brAfter = await cdpProgram.account.borrowRewards.fetch(brBorrowRewards);
      const posAfter = await cdpProgram.account.cdpPosition.fetch(brPosition);
      const cfgAfter = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);

      // principal should have increased
      assert.equal(
        posAfter.riseSolDebtPrincipal.toNumber(),
        posBefore.riseSolDebtPrincipal.toNumber() + additionalBorrow
      );

      // totalCdpDebt should also have increased
      assert.equal(
        cfgAfter.totalCdpDebt.toNumber(),
        cfgBefore.totalCdpDebt.toNumber() + additionalBorrow
      );

      // reward_debt should reflect new debt * reward_per_token
      const expectedDebt = (BigInt(posAfter.riseSolDebtPrincipal.toString()) *
        BigInt(cfgAfter.rewardPerToken.toString())) /
        BigInt("1000000000000");
      assert.equal(
        brAfter.rewardDebt.toString(),
        expectedDebt.toString(),
        "reward_debt should equal new_debt * reward_per_token / REWARD_SCALE"
      );

      console.log("borrow_more: principal", posBefore.riseSolDebtPrincipal.toString(),
        "→", posAfter.riseSolDebtPrincipal.toString());
      console.log("reward_debt:", brBefore.rewardDebt.toString(), "→", brAfter.rewardDebt.toString());
    });

    it("Claim transfers correct RISE amount to borrower", async () => {
      // Mint some RISE tokens directly to the rewards vault so the claim can succeed
      await mintTo(
        provider.connection,
        authority.payer,
        riseMint,
        borrowRewardsVault,
        authority.publicKey,
        10_000 * 1_000_000  // 10,000 RISE
      );

      // Advance the accumulator by checkpointing again
      await cdpProgram.methods
        .checkpointBorrowRewards()
        .accounts({
          caller: authority.publicKey,
          borrowRewardsConfig,
        })
        .rpc();

      const brBefore = await cdpProgram.account.borrowRewards.fetch(brBorrowRewards);
      const riseBefore = (await getAccount(provider.connection, brUserRiseAccount)).amount;

      await cdpProgram.methods
        .claimBorrowRewards(NONCE_BR)
        .accounts({
          borrower: authority.publicKey,
          position: brPosition,
          borrowRewards: brBorrowRewards,
          borrowRewardsConfig,
          rewardsVault: borrowRewardsVault,
          borrowerRiseAccount: brUserRiseAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      const brAfter = await cdpProgram.account.borrowRewards.fetch(brBorrowRewards);
      const riseAfter = (await getAccount(provider.connection, brUserRiseAccount)).amount;

      assert.equal(brAfter.pendingRewards.toNumber(), 0, "pending_rewards should be 0 after claim");
      assert.isTrue(
        brAfter.totalClaimed.toNumber() > 0,
        "total_claimed should be > 0 after claim"
      );
      assert.isTrue(
        riseAfter > riseBefore,
        "Borrower RISE balance should increase after claim"
      );

      console.log("RISE claimed:", Number(riseAfter - riseBefore), "units");
      console.log("total_claimed:", brAfter.totalClaimed.toString());
    });

    it("Repay settles rewards before reducing debt", async () => {
      const [cdpFeeVault] = PublicKey.findProgramAddressSync(
        [Buffer.from("cdp_fee_vault")],
        cdpProgram.programId
      );
      const [poolVault] = PublicKey.findProgramAddressSync(
        [Buffer.from("pool_vault")],
        stakingProgram.programId
      );
      const [solPaymentConfig] = PublicKey.findProgramAddressSync(
        [Buffer.from("payment_config"), anchor.web3.SystemProgram.programId.toBuffer()],
        cdpProgram.programId
      );

      const brBefore = await cdpProgram.account.borrowRewards.fetch(brBorrowRewards);
      const posBefore = await cdpProgram.account.cdpPosition.fetch(brPosition);
      const cfgBefore = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);

      // Partial repay with 0.5 SOL
      const paymentLamports = 0.5 * anchor.web3.LAMPORTS_PER_SOL;

      const { priceUpdateKeypair: puKp9, solPriceUpdateKeypair: spuKp9, priceUpdateIx: puIx9, solPriceUpdateIx: spuIx9 } =
        await buildCdpPriceUpdateIxs(provider.connection, authority.publicKey, USDC_FEED_ID_HEX);

      await cdpProgram.methods
        .repayDebt(new anchor.BN(paymentLamports))
        .accounts({
          borrower: authority.publicKey,
          position: brPosition,
          collateralConfig,
          paymentConfig: solPaymentConfig,
          globalPool,
          cdpConfig,
          cdpFeeVault,
          poolVault,
          collateralVault,
          borrowerCollateralAccount: userUsdcAccount,
          priceUpdate: puKp9.publicKey,
          solPriceUpdate: spuKp9.publicKey,
          paymentMint: null,
          borrowerPaymentAccount: null,
          paymentVault: null,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          borrowRewardsConfig,
          borrowRewards: brBorrowRewards,
        })
        .preInstructions([puIx9, spuIx9])
        .signers([puKp9, spuKp9])
        .rpc();

      const brAfter = await cdpProgram.account.borrowRewards.fetch(brBorrowRewards);
      const posAfter = await cdpProgram.account.cdpPosition.fetch(brPosition);
      const cfgAfter = await cdpProgram.account.borrowRewardsConfig.fetch(borrowRewardsConfig);

      // principal should have decreased
      assert.isTrue(
        posAfter.riseSolDebtPrincipal.toNumber() < posBefore.riseSolDebtPrincipal.toNumber(),
        "Principal should decrease after partial repay"
      );

      // totalCdpDebt should also have decreased
      assert.isTrue(
        cfgAfter.totalCdpDebt.toNumber() <= cfgBefore.totalCdpDebt.toNumber(),
        "totalCdpDebt should not increase after repay"
      );

      // reward_debt should reflect new lower debt
      const expectedDebt = (BigInt(posAfter.riseSolDebtPrincipal.toString()) *
        BigInt(cfgAfter.rewardPerToken.toString())) /
        BigInt("1000000000000");
      assert.equal(
        brAfter.rewardDebt.toString(),
        expectedDebt.toString(),
        "reward_debt should reflect reduced debt after repay"
      );

      console.log("Repay: principal", posBefore.riseSolDebtPrincipal.toString(),
        "→", posAfter.riseSolDebtPrincipal.toString());
      console.log("totalCdpDebt:", cfgBefore.totalCdpDebt.toString(),
        "→", cfgAfter.totalCdpDebt.toString());
    });
  });

  describe("collect_cdp_fees", () => {
    it("Collects accumulated CDP fees from cdp_fee_vault", async () => {
      const [cdpFeeVault] = PublicKey.findProgramAddressSync(
        [Buffer.from("cdp_fee_vault")],
        cdpProgram.programId
      );
      const [poolVault] = PublicKey.findProgramAddressSync(
        [Buffer.from("pool_vault")],
        stakingProgram.programId
      );
      const [treasuryVault] = PublicKey.findProgramAddressSync(
        [Buffer.from("treasury_vault")],
        stakingProgram.programId
      );
      const [stakingTreasury] = PublicKey.findProgramAddressSync(
        [Buffer.from("protocol_treasury")],
        stakingProgram.programId
      );

      const feeVaultBalance = await provider.connection.getBalance(cdpFeeVault);
      if (feeVaultBalance === 0) {
        console.log("cdp_fee_vault is empty — no fees to collect, skipping");
        return;
      }

      const poolVaultBalanceBefore = await provider.connection.getBalance(poolVault);
      const treasury = await stakingProgram.account.protocolTreasury.fetch(stakingTreasury);
      const indexBefore = treasury.revenueIndex;

      await cdpProgram.methods
        .collectCdpFees()
        .accounts({
          caller: authority.publicKey,
          cdpFeeVault,
          treasury: stakingTreasury,
          treasuryVault,
          globalPool,
          poolVault,
          stakingProgram: stakingProgram.programId,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      // CDP fee split: 90% → pool_vault (stakers), 5% → treasury reserve, 5% → veRISE
      const poolVaultBalanceAfter = await provider.connection.getBalance(poolVault);
      assert.isTrue(
        poolVaultBalanceAfter > poolVaultBalanceBefore,
        "Pool vault should receive 90% of CDP fees"
      );

      const treasuryAfter = await stakingProgram.account.protocolTreasury.fetch(stakingTreasury);
      assert.isTrue(
        treasuryAfter.revenueIndex.gt(indexBefore),
        "Revenue index should increase (veRISE share registered)"
      );

      console.log("CDP fees collected");
      console.log(
        "Pool vault gained:",
        poolVaultBalanceAfter - poolVaultBalanceBefore,
        "lamports (90%)"
      );
      console.log(
        "Revenue index:",
        indexBefore.toString(),
        "→",
        treasuryAfter.revenueIndex.toString()
      );
    });
  });
});
