/**
 * smoke_test.mjs
 *
 * Devnet smoke test covering all four Rise Protocol programs.
 *
 * READ-ONLY checks: verify all config/vault PDAs are initialized and have
 * sensible state.
 *
 * TRANSACTION test: stake 0.01 SOL → riseSOL (cheapest possible proof that the
 * staking program is live and the on-chain math is working).
 *
 * Run from rise-protocol/:  node smoke_test.mjs
 */

import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet, BN } = pkg;
import {
  Connection,
  Keypair,
  PublicKey,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  getAccount,
} from "@solana/spl-token";
import { readFileSync } from "fs";
import { homedir } from "os";
import path from "path";

// ── Constants ─────────────────────────────────────────────────────────────────
const RPC             = "https://devnet.helius-rpc.com/?api-key=787be2ec-9299-40c2-af00-e559a4715fa1";
const STAKING_PROG_ID = new PublicKey("BnQc6jJMT6mt3mvWuQFAd9vf2T2wWkAYD2uGjCXud6Lo");
const CDP_PROG_ID     = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");
const GOV_PROG_ID     = new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");
const REWARDS_PROG_ID = new PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8");

const RISE_SOL_MINT = new PublicKey("86bHg3K32cRhnfcYTr3RCgKZme4xSLZzMyzWA8qDswHp");
const RISE_MINT     = new PublicKey("2TysJ9Tw5WLh7hBLmC6iZp73bm6akogYEushJEf8K49Q");
const WSOL_MINT     = new PublicKey("So11111111111111111111111111111111111111112");

const STAKE_AMOUNT_LAMPORTS = 10_000_000; // 0.01 SOL

// ── Setup ─────────────────────────────────────────────────────────────────────
const keyPath = path.join(homedir(), ".config/solana/id.json");
const keypair = Keypair.fromSecretKey(
  Uint8Array.from(JSON.parse(readFileSync(keyPath, "utf8")))
);

const connection = new Connection(RPC, "confirmed");
const wallet     = new Wallet(keypair);
const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });

const loadIdl = (name) =>
  JSON.parse(readFileSync(
    new URL(`./target/idl/${name}.json`, import.meta.url).pathname, "utf8"
  ));

const staking  = new Program(loadIdl("rise_staking"),  provider);
const cdp      = new Program(loadIdl("rise_cdp"),      provider);
const gov      = new Program(loadIdl("rise_governance"), provider);
const rewards  = new Program(loadIdl("rise_rewards"),  provider);

// ── PDA derivations ───────────────────────────────────────────────────────────
const [pool]             = PublicKey.findProgramAddressSync([Buffer.from("global_pool")],             STAKING_PROG_ID);
const [poolVault]        = PublicKey.findProgramAddressSync([Buffer.from("pool_vault")],              STAKING_PROG_ID);
const [stakingTreasury]  = PublicKey.findProgramAddressSync([Buffer.from("protocol_treasury")],       STAKING_PROG_ID);

const [cdpConfig]             = PublicKey.findProgramAddressSync([Buffer.from("cdp_config")],             CDP_PROG_ID);
const [cdpWsolVault]          = PublicKey.findProgramAddressSync([Buffer.from("cdp_wsol_vault")],         CDP_PROG_ID);
const [cdpWsolBuybackVault]   = PublicKey.findProgramAddressSync([Buffer.from("cdp_wsol_buyback_vault")], CDP_PROG_ID);
const [collateralConfig]      = PublicKey.findProgramAddressSync(
  [Buffer.from("collateral_config"), RISE_SOL_MINT.toBuffer()], CDP_PROG_ID
);
const [collateralVault]       = PublicKey.findProgramAddressSync(
  [Buffer.from("collateral_vault"), RISE_SOL_MINT.toBuffer()], CDP_PROG_ID
);

const [govConfig]  = PublicKey.findProgramAddressSync([Buffer.from("governance_config")], GOV_PROG_ID);
const [riseVault]  = PublicKey.findProgramAddressSync([Buffer.from("rise_vault")],        GOV_PROG_ID);

const [rewardsConfig] = PublicKey.findProgramAddressSync([Buffer.from("rewards_config")], REWARDS_PROG_ID);

// ── Helpers ───────────────────────────────────────────────────────────────────
let passed = 0;
let failed = 0;

function ok(label, value) {
  console.log(`  ✓ ${label}: ${value}`);
  passed++;
}

function fail(label, reason) {
  console.error(`  ✗ ${label}: ${reason}`);
  failed++;
}

async function checkAccount(label, pubkey) {
  const info = await connection.getAccountInfo(pubkey);
  if (info) {
    ok(label, `${pubkey.toBase58()} (${info.lamports / LAMPORTS_PER_SOL} SOL)`);
    return true;
  } else {
    fail(label, `${pubkey.toBase58()} — NOT FOUND`);
    return false;
  }
}

// ── Test sections ─────────────────────────────────────────────────────────────

async function testStakingAccounts() {
  console.log("\n[1/5] Staking program accounts");

  await checkAccount("GlobalPool", pool);
  await checkAccount("ProtocolTreasury", stakingTreasury);
  await checkAccount("pool_vault (SOL)", poolVault);

  try {
    const poolData = await staking.account.globalPool.fetch(pool);
    ok("riseSOL supply", `${poolData.stakingRiseSolSupply.toString()} (raw)`);
    ok("total_sol_staked", `${poolData.totalSolStaked.toString()} lamports`);
    ok("exchange_rate", `${poolData.exchangeRate.toString()}`);
  } catch (e) {
    fail("GlobalPool.fetch", e.message);
  }

  try {
    const treasury = await staking.account.protocolTreasury.fetch(stakingTreasury);
    ok("team_wallet", treasury.teamWallet.toBase58());
  } catch (e) {
    fail("ProtocolTreasury.fetch", e.message);
  }
}

async function testCdpAccounts() {
  console.log("\n[2/5] CDP program accounts");

  await checkAccount("CdpConfig", cdpConfig);
  await checkAccount("cdp_wsol_vault", cdpWsolVault);
  await checkAccount("cdp_wsol_buyback_vault", cdpWsolBuybackVault);
  await checkAccount("riseSOL collateral_config", collateralConfig);
  await checkAccount("riseSOL collateral_vault", collateralVault);

  try {
    const config = await cdp.account.cdpConfig.fetch(cdpConfig);
    ok("cdp_rise_sol_minted", `${config.cdpRiseSolMinted.toString()} (raw)`);
    ok("debt_ceiling_multiplier_bps", config.debtCeilingMultiplierBps.toString());
  } catch (e) {
    fail("CdpConfig.fetch", e.message);
  }

  try {
    const cc = await cdp.account.collateralConfig.fetch(collateralConfig);
    ok("riseSOL max_ltv_bps", cc.maxLtvBps.toString());
    ok("riseSOL pyth_price_feed", cc.pythPriceFeed.toBase58());
    ok("riseSOL active", cc.active.toString());
  } catch (e) {
    fail("CollateralConfig.fetch", e.message);
  }
}

async function testGovernanceAccounts() {
  console.log("\n[3/5] Governance program accounts");

  await checkAccount("GovernanceConfig", govConfig);
  await checkAccount("rise_vault", riseVault);

  try {
    const config = await gov.account.governanceConfig.fetch(govConfig);
    if (config.riseMint.equals(RISE_MINT)) {
      ok("rise_mint matches rewards mint", "yes");
    } else {
      fail("rise_mint MISMATCH", `gov=${config.riseMint.toBase58()} rewards=${RISE_MINT.toBase58()}`);
    }
    ok("proposal_count", config.proposalCount.toString());
    ok("quorum_bps", config.quorumBps.toString());
  } catch (e) {
    fail("GovernanceConfig.fetch", e.message);
  }
}

async function testRewardsAccounts() {
  console.log("\n[4/5] Rewards program accounts");

  await checkAccount("RewardsConfig", rewardsConfig);

  try {
    const config = await rewards.account.rewardsConfig.fetch(rewardsConfig);
    ok("gauge_count", config.gaugeCount.toString());
    ok("epoch_emissions", config.epochEmissions.toString());
    ok("rise_mint matches", config.riseMint.equals(RISE_MINT) ? "yes" : `MISMATCH: ${config.riseMint}`);

    const gaugeCount = config.gaugeCount.toNumber();
    const allGauges  = await rewards.account.gauge.all();
    const gauges     = allGauges.filter(g => g.account.index.toNumber() < gaugeCount);
    ok(`gauge accounts (${gauges.length}/${gaugeCount})`,
      gauges.map(g => `#${g.account.index.toNumber()} pool=${g.account.pool.toBase58().slice(0,8)}…`).join(", "));
  } catch (e) {
    fail("RewardsConfig.fetch / gauges", e.message);
  }
}

async function testStakeTransaction() {
  console.log("\n[5/5] Stake transaction (0.01 SOL → riseSOL)");

  // Check wallet balance first
  const balance = await connection.getBalance(keypair.publicKey);
  console.log(`  Wallet balance: ${(balance / LAMPORTS_PER_SOL).toFixed(4)} SOL`);

  if (balance < STAKE_AMOUNT_LAMPORTS + 10_000_000) {
    fail("stake_sol", "insufficient balance (need ≥ 0.02 SOL)");
    return;
  }

  const userRiseSolAta = getAssociatedTokenAddressSync(
    RISE_SOL_MINT, keypair.publicKey
  );

  // Create ATA if needed
  const ataInfo = await connection.getAccountInfo(userRiseSolAta);
  if (!ataInfo) {
    console.log("  Creating riseSOL ATA...");
    const tx = new (await import("@solana/web3.js")).Transaction().add(
      createAssociatedTokenAccountInstruction(
        keypair.publicKey, userRiseSolAta, keypair.publicKey, RISE_SOL_MINT
      )
    );
    await provider.sendAndConfirm(tx, [keypair]);
    console.log("  ATA created.");
  }

  // Register user_stake_rewards if not yet initialized (needed for stake_sol optional account)
  const [userStakeRewards] = PublicKey.findProgramAddressSync(
    [Buffer.from("user_stake_rewards"), keypair.publicKey.toBuffer()], STAKING_PROG_ID
  );
  const [stakeRewardsConfig] = PublicKey.findProgramAddressSync(
    [Buffer.from("stake_rewards_config")], STAKING_PROG_ID
  );
  const usrInfo = await connection.getAccountInfo(userStakeRewards);
  if (!usrInfo) {
    console.log("  Registering user_stake_rewards...");
    try {
      const tx = await staking.methods
        .registerStakeRewards()
        .accounts({
          user:              keypair.publicKey,
          pool,
          stakeRewardsConfig,
          userStakeRewards,
          userRiseSolAccount: userRiseSolAta,
        })
        .rpc();
      console.log("  Registered. Tx:", tx);
    } catch (e) {
      fail("register_stake_rewards", e.message);
      return;
    }
  }

  const balanceBefore = await connection.getTokenAccountBalance(userRiseSolAta).catch(() => ({ value: { uiAmount: 0 } }));

  try {
    const tx = await staking.methods
      .stakeSol(new BN(STAKE_AMOUNT_LAMPORTS))
      .accounts({
        user:                keypair.publicKey,
        pool,
        poolVault,
        riseSolMint:         RISE_SOL_MINT,
        userRiseSolAccount:  userRiseSolAta,
        stakeRewardsConfig,
        userStakeRewards,
      })
      .rpc();

    const balanceAfter = await connection.getTokenAccountBalance(userRiseSolAta);
    const minted = (balanceAfter.value.uiAmount ?? 0) - (balanceBefore.value.uiAmount ?? 0);

    ok("stake_sol tx", tx);
    ok("riseSOL received", `+${minted.toFixed(6)} riseSOL`);
  } catch (e) {
    fail("stake_sol tx", e.message);
    if (e.logs) console.error("  Logs:", e.logs.join("\n  "));
  }
}

// ── Main ──────────────────────────────────────────────────────────────────────
console.log("Rise Protocol — Devnet Smoke Test");
console.log("Authority:", keypair.publicKey.toBase58());

await testStakingAccounts();
await testCdpAccounts();
await testGovernanceAccounts();
await testRewardsAccounts();
await testStakeTransaction();

console.log("\n──────────────────────────────────────────");
console.log(`Results: ${passed} passed, ${failed} failed`);
if (failed > 0) {
  console.error("SMOKE TEST FAILED");
  process.exit(1);
} else {
  console.log("ALL CHECKS PASSED");
}
