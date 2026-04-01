/**
 * set_gauge_weights.mjs
 *
 * Sets initial gauge weights. Adjust WEIGHTS below to match vote tally each epoch.
 * Must sum to 10,000 bps.
 *
 * Run from rise-protocol/:  node set_gauge_weights.mjs
 */

import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet } = pkg;
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "fs";
import { homedir } from "os";
import path from "path";

const RPC             = "https://devnet.helius-rpc.com/?api-key=787be2ec-9299-40c2-af00-e559a4715fa1";
const REWARDS_PROG_ID = new PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8");

const makePlaceholder = (seed) =>
  Keypair.fromSeed(Uint8Array.from(Buffer.from(seed.padEnd(32, "\0")))).publicKey;

// Weights must sum to 10,000 bps
const GAUGE_WEIGHTS = [
  { label: "riseSOL/SOL  (Orca)",    pool: makePlaceholder("rise-pool-risesol-sol"),  weightBps: 4000 },
  { label: "riseSOL/USDC (Orca)",    pool: makePlaceholder("rise-pool-risesol-usdc"), weightBps: 3000 },
  { label: "RISE/SOL     (Raydium)", pool: makePlaceholder("rise-pool-rise-sol"),      weightBps: 3000 },
];

const total = GAUGE_WEIGHTS.reduce((s, g) => s + g.weightBps, 0);
if (total !== 10_000) { console.error(`Weights sum to ${total}, must be 10000`); process.exit(1); }

const keyPath = path.join(homedir(), ".config/solana/id.json");
const keypair = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(keyPath, "utf8"))));
const idl     = JSON.parse(readFileSync(new URL("./target/idl/rise_rewards.json", import.meta.url).pathname, "utf8"));

const connection = new Connection(RPC, "confirmed");
const provider   = new AnchorProvider(connection, new Wallet(keypair), { commitment: "confirmed" });
const program    = new Program(idl, provider);

const [config] = PublicKey.findProgramAddressSync([Buffer.from("rewards_config")], REWARDS_PROG_ID);

for (const { label, pool, weightBps } of GAUGE_WEIGHTS) {
  const [gauge] = PublicKey.findProgramAddressSync(
    [Buffer.from("gauge"), pool.toBuffer()],
    REWARDS_PROG_ID,
  );
  try {
    const tx = await program.methods
      .setGaugeWeight(weightBps)
      .accounts({ authority: keypair.publicKey, config, gauge })
      .rpc();
    console.log(`${label}: ${weightBps} bps — Tx: ${tx.slice(0, 16)}...`);
  } catch (err) {
    console.error(`Failed for ${label}:`, err.message);
  }
}

console.log("Done.");
