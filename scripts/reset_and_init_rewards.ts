/**
 * Updates borrow-rewards and LP-rewards emission parameters.
 *
 * - Borrow rewards (CDP): closes and reinitializes (no set_epoch_emissions instruction exists).
 * - LP rewards: uses set_epoch_emissions — the rewards_config and all gauges are left untouched.
 *
 * Run with: npx ts-node scripts/reset_and_init_rewards.ts
 */
import * as anchor from "@coral-xyz/anchor";
import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";

// ── Config ────────────────────────────────────────────────────────────────────

const RPC        = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const RISE_MINT  = new PublicKey("2TysJ9Tw5WLh7hBLmC6iZp73bm6akogYEushJEf8K49Q");

const CDP_PROGRAM_ID     = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");
const REWARDS_PROGRAM_ID = new PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8");

// Emission parameters — total weekly: ~3,807,692 RISE
//   Stakers (40%): ~1,523,077 RISE/week  — set via initialize_stake_rewards.ts
//   Borrowers (30%): ~1,142,308 RISE/week
//   LP providers (30%): ~1,142,308 RISE/week
const CDP_EPOCH_EMISSIONS  = BigInt("1142308000000"); // 1,142,308 RISE (6 decimals)
const CDP_SLOTS_PER_EPOCH  = BigInt("604800");        // ~1 week
const LP_EPOCH_EMISSIONS   = BigInt("1142308000000"); // 1,142,308 RISE (6 decimals)

const KEYPAIR_PATH = process.env.ANCHOR_WALLET ?? `${process.env.HOME}/.config/solana/id.json`;

// ── PDA helpers ───────────────────────────────────────────────────────────────

function pda(seeds: Buffer[], programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(seeds, programId)[0];
}

const cdpConfig            = pda([Buffer.from("cdp_config")],            CDP_PROGRAM_ID);
const borrowRewardsConfig  = pda([Buffer.from("borrow_rewards_config")], CDP_PROGRAM_ID);
const borrowRewardsVault   = pda([Buffer.from("borrow_rewards_vault")],  CDP_PROGRAM_ID);
const rewardsConfig        = pda([Buffer.from("rewards_config")],        REWARDS_PROGRAM_ID);
const rewardsVault         = pda([Buffer.from("rewards_vault")],         REWARDS_PROGRAM_ID);

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  const connection = new Connection(RPC, "confirmed");
  const raw        = JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf-8"));
  const payer      = Keypair.fromSecretKey(Uint8Array.from(raw));
  const wallet     = new Wallet(payer);
  const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  const cdpIdl     = JSON.parse(fs.readFileSync(path.join(__dirname, "../target/idl/rise_cdp.json"),     "utf-8"));
  const rewardsIdl = JSON.parse(fs.readFileSync(path.join(__dirname, "../target/idl/rise_rewards.json"), "utf-8"));
  const cdp     = new Program(cdpIdl,     provider) as any;
  const rewards = new Program(rewardsIdl, provider) as any;

  console.log("Authority:", payer.publicKey.toBase58());
  console.log("RISE mint:", RISE_MINT.toBase58());
  console.log();

  // ── 1. Borrow rewards (CDP): close + reinitialize ─────────────────────────────
  const borrowConfigInfo = await connection.getAccountInfo(borrowRewardsConfig);
  if (borrowConfigInfo) {
    const oldMint = new PublicKey(borrowConfigInfo.data.slice(40, 72));
    console.log("Closing borrow_rewards_config (old mint:", oldMint.toBase58() + ")...");
    const sig = await cdp.methods
      .closeBorrowRewards()
      .accounts({
        authority:    payer.publicKey,
        cdpConfig,
        borrowRewardsConfig,
        rewardsVault: borrowRewardsVault,
        oldRiseMint:  oldMint,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
    console.log("[OK] close_borrow_rewards:", sig.slice(0, 20) + "...");
  } else {
    console.log("[SKIP] borrow_rewards_config not found — will initialize fresh");
  }

  console.log("Initializing borrow rewards...");
  console.log(`  epoch_emissions: ${CDP_EPOCH_EMISSIONS} (${Number(CDP_EPOCH_EMISSIONS) / 1e6} RISE/epoch)`);
  const sig2 = await cdp.methods
    .initializeBorrowRewards(
      new anchor.BN(CDP_EPOCH_EMISSIONS.toString()),
      new anchor.BN(CDP_SLOTS_PER_EPOCH.toString()),
    )
    .accounts({
      authority:           payer.publicKey,
      cdpConfig,
      borrowRewardsConfig,
      rewardsVault:        borrowRewardsVault,
      riseMint:            RISE_MINT,
      tokenProgram:        TOKEN_PROGRAM_ID,
      systemProgram:       SystemProgram.programId,
    })
    .rpc();
  console.log("[OK] initialize_borrow_rewards:", sig2.slice(0, 20) + "...");
  console.log();

  // ── 2. LP rewards: set_epoch_emissions (gauges untouched) ────────────────────
  const rewardsConfigInfo = await connection.getAccountInfo(rewardsConfig);
  if (rewardsConfigInfo) {
    console.log("Updating LP rewards epoch_emissions via set_epoch_emissions...");
    console.log(`  epoch_emissions: ${LP_EPOCH_EMISSIONS} (${Number(LP_EPOCH_EMISSIONS) / 1e6} RISE/epoch)`);
    const sig = await rewards.methods
      .setEpochEmissions(new anchor.BN(LP_EPOCH_EMISSIONS.toString()))
      .accounts({
        authority: payer.publicKey,
        config:    rewardsConfig,
      })
      .rpc();
    console.log("[OK] set_epoch_emissions:", sig.slice(0, 20) + "...");
  } else {
    // First time — initialize from scratch including vault
    console.log("Initializing LP rewards config (first time)...");
    console.log(`  epoch_emissions: ${LP_EPOCH_EMISSIONS} (${Number(LP_EPOCH_EMISSIONS) / 1e6} RISE/epoch)`);
    const sig = await rewards.methods
      .initializeRewards(new anchor.BN(LP_EPOCH_EMISSIONS.toString()))
      .accounts({
        authority:     payer.publicKey,
        config:        rewardsConfig,
        riseMint:      RISE_MINT,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    console.log("[OK] initialize_rewards:", sig.slice(0, 20) + "...");

    console.log("Initializing LP rewards vault...");
    const sig2 = await rewards.methods
      .initializeRewardsVault()
      .accounts({
        authority:     payer.publicKey,
        config:        rewardsConfig,
        rewardsVault,
        riseMint:      RISE_MINT,
        tokenProgram:  TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    console.log("[OK] initialize_rewards_vault:", sig2.slice(0, 20) + "...");
  }

  console.log();
  console.log("CDP borrow_rewards_vault: ", borrowRewardsVault.toBase58());
  console.log("LP  rewards_vault:        ", rewardsVault.toBase58());
  console.log("\nDone.");
}

main().catch((e) => { console.error(e); process.exit(1); });
