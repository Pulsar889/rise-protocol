/**
 * reset_rewards.mjs
 *
 * 1. Closes the existing RewardsConfig (reclaims rent, wipes gauge_count)
 * 2. Re-initializes RewardsConfig with the same settings
 * 3. Closes ALL existing Gauge accounts (prevents accumulation across resets)
 * 4. Creates 3 gauges (riseSOL/SOL, riseSOL/USDC, RISE/SOL)
 *
 * Steps 1 and 3 are critical: without closing old gauges before creating new
 * ones, orphaned gauge accounts accumulate on-chain and the frontend shows
 * them all (gauge.all() returns every Gauge owned by the program).
 *
 * The rewards_vault token account is NOT touched — it already exists and is
 * still valid because the rewards_config PDA address is deterministic.
 *
 * Gauge PDAs are seeded with a placeholder pool pubkey. Update GAUGE_POOLS
 * below to real Orca/Raydium pool addresses when those pools are created.
 *
 * Run from rise-protocol/:  node reset_rewards.mjs
 */

import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet, BN } = pkg;
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "fs";
import { homedir } from "os";
import path from "path";

const RPC             = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const REWARDS_PROG_ID = new PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8");
const RISE_MINT       = new PublicKey("2TysJ9Tw5WLh7hBLmC6iZp73bm6akogYEushJEf8K49Q");

// ── Placeholder pool pubkeys ──────────────────────────────────────────────────
// Replace with real Orca / Raydium pool addresses when pools are created.
// These are deterministic placeholders derived from a fixed seed so they are
// stable across runs. Swap them for real pool addresses before mainnet.
const makePlaceholder = (seed) =>
  Keypair.fromSeed(Uint8Array.from(Buffer.from(seed.padEnd(32, "\0")))).publicKey;

const GAUGE_POOLS = {
  "riseSOL/SOL  (Orca)":    makePlaceholder("rise-pool-risesol-sol"),
  "riseSOL/USDC (Orca)":    makePlaceholder("rise-pool-risesol-usdc"),
  "RISE/SOL     (Raydium)": makePlaceholder("rise-pool-rise-sol"),
};

// Epoch emissions: 100,000 RISE per epoch (9 decimals → 100_000 * 1e9)
const EPOCH_EMISSIONS = new BN("100000000000000");

// ─────────────────────────────────────────────────────────────────────────────

const keyPath = path.join(homedir(), ".config/solana/id.json");
const secret  = JSON.parse(readFileSync(keyPath, "utf8"));
const keypair = Keypair.fromSecretKey(Uint8Array.from(secret));

const idlPath = new URL("./target/idl/rise_rewards.json", import.meta.url).pathname;
const idl     = JSON.parse(readFileSync(idlPath, "utf8"));

const connection = new Connection(RPC, "confirmed");
const wallet     = new Wallet(keypair);
const provider   = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
const program    = new Program(idl, provider);

const [config] = PublicKey.findProgramAddressSync([Buffer.from("rewards_config")], REWARDS_PROG_ID);

console.log("Authority:", keypair.publicKey.toBase58());
console.log("RewardsConfig PDA:", config.toBase58());

// ── Step 1: Close existing config ─────────────────────────────────────────────
console.log("\n[1/4] Closing existing RewardsConfig...");
try {
  const tx = await program.methods
    .closeRewardsConfig()
    .accounts({
      authority: keypair.publicKey,
      config,
    })
    .rpc();
  console.log("  Closed. Tx:", tx);
} catch (err) {
  if (err.message?.includes("AccountNotInitialized") || err.message?.includes("AccountNotFound")) {
    console.log("  No existing config found — skipping close.");
  } else {
    console.error("  Failed to close config:", err.message);
    process.exit(1);
  }
}

// ── Step 2: Re-initialize config ─────────────────────────────────────────────
console.log("\n[2/4] Re-initializing RewardsConfig...");
try {
  const tx = await program.methods
    .initializeRewards(EPOCH_EMISSIONS)
    .accounts({
      authority:   keypair.publicKey,
      config,
      riseMint:    RISE_MINT,
      systemProgram: new PublicKey("11111111111111111111111111111111"),
    })
    .rpc();
  console.log("  Initialized. Tx:", tx);
} catch (err) {
  console.error("  Failed to initialize rewards:", err.message);
  if (err.logs) console.error(err.logs);
  process.exit(1);
}

// ── Step 3: Close ALL existing gauge accounts ─────────────────────────────────
// Must happen after config is re-initialized (close_gauge requires config for
// authority validation). Prevents orphaned gauges from accumulating on-chain.
console.log("\n[3/4] Closing all existing gauge accounts...");
const existingGauges = await program.account.gauge.all();
if (existingGauges.length === 0) {
  console.log("  No existing gauges found.");
} else {
  console.log(`  Found ${existingGauges.length} gauge(s) to close.`);
  for (const { publicKey: gaugePda, account: gaugeAcc } of existingGauges) {
    try {
      const tx = await program.methods
        .closeGauge()
        .accounts({
          authority: keypair.publicKey,
          config,
          gauge:     gaugePda,
        })
        .rpc();
      console.log(`  Closed gauge #${gaugeAcc.index} (${gaugePda.toBase58()}). Tx: ${tx}`);
    } catch (err) {
      console.error(`  Failed to close gauge ${gaugePda.toBase58()}:`, err.message);
      if (err.logs) console.error(err.logs);
      process.exit(1);
    }
  }
}

// ── Step 4: Create 3 gauges ───────────────────────────────────────────────────
console.log("\n[4/4] Creating gauges...");
let index = 0;
for (const [label, pool] of Object.entries(GAUGE_POOLS)) {
  const [gaugePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("gauge"), pool.toBuffer()],
    REWARDS_PROG_ID,
  );
  try {
    const tx = await program.methods
      .createGauge(pool)
      .accounts({
        authority:    keypair.publicKey,
        config,
        gauge:        gaugePda,
        systemProgram: new PublicKey("11111111111111111111111111111111"),
      })
      .rpc();
    console.log(`  Gauge #${index} — ${label}`);
    console.log(`    PDA:  ${gaugePda.toBase58()}`);
    console.log(`    Pool: ${pool.toBase58()}`);
    console.log(`    Tx:   ${tx}`);
    index++;
  } catch (err) {
    console.error(`  Failed to create gauge ${label}:`, err.message);
    if (err.logs) console.error(err.logs);
    process.exit(1);
  }
}

console.log("\nDone. 3 gauges created.");
console.log("Update GAUGE_POOLS in this script with real pool addresses when ready.");
