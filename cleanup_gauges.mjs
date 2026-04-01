/**
 * cleanup_gauges.mjs
 *
 * 1. Closes all orphaned Gauge accounts on-chain
 * 2. Creates 3 fresh gauges (riseSOL/SOL, riseSOL/USDC, RISE/SOL)
 *
 * Run from rise-protocol/:  node cleanup_gauges.mjs
 */

import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet, BN } = pkg;
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "fs";
import { homedir } from "os";
import path from "path";

const RPC             = "https://devnet.helius-rpc.com/?api-key=787be2ec-9299-40c2-af00-e559a4715fa1";
const REWARDS_PROG_ID = new PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8");

const makePlaceholder = (seed) =>
  Keypair.fromSeed(Uint8Array.from(Buffer.from(seed.padEnd(32, "\0")))).publicKey;

const GAUGE_POOLS = [
  { label: "riseSOL/SOL  (Orca)",    pool: makePlaceholder("rise-pool-risesol-sol")  },
  { label: "riseSOL/USDC (Orca)",    pool: makePlaceholder("rise-pool-risesol-usdc") },
  { label: "RISE/SOL     (Raydium)", pool: makePlaceholder("rise-pool-rise-sol")      },
];

const keyPath = path.join(homedir(), ".config/solana/id.json");
const keypair = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(keyPath, "utf8"))));
const idl     = JSON.parse(readFileSync(new URL("./target/idl/rise_rewards.json", import.meta.url).pathname, "utf8"));

const connection = new Connection(RPC, "confirmed");
const provider   = new AnchorProvider(connection, new Wallet(keypair), { commitment: "confirmed" });
const program    = new Program(idl, provider);

const [config] = PublicKey.findProgramAddressSync([Buffer.from("rewards_config")], REWARDS_PROG_ID);

// ── Step 1: Close all existing gauge accounts ─────────────────────────────────
console.log("[1/2] Fetching and closing all gauge accounts...");
const allGauges = await program.account.gauge.all();
console.log(`  Found ${allGauges.length} gauge(s) to close.`);

for (const g of allGauges) {
  try {
    const tx = await program.methods
      .closeGauge()
      .accounts({
        authority: keypair.publicKey,
        config,
        gauge: g.publicKey,
      })
      .rpc();
    console.log(`  Closed gauge #${g.account.index} (${g.publicKey.toBase58().slice(0,8)}...) Tx: ${tx.slice(0,16)}...`);
  } catch (err) {
    console.error(`  Failed to close gauge #${g.account.index}:`, err.message);
  }
}

// ── Step 2: Create 3 fresh gauges ─────────────────────────────────────────────
console.log("\n[2/2] Creating 3 fresh gauges...");
for (let i = 0; i < GAUGE_POOLS.length; i++) {
  const { label, pool } = GAUGE_POOLS[i];
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
    console.log(`  Gauge #${i} — ${label}`);
    console.log(`    PDA:  ${gaugePda.toBase58()}`);
    console.log(`    Tx:   ${tx}`);
  } catch (err) {
    console.error(`  Failed to create gauge ${label}:`, err.message);
    if (err.logs) console.error(err.logs);
  }
}

console.log("\nDone.");
