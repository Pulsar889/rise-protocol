/**
 * tally_and_set_weights.mjs
 *
 * 1. Reads all GaugeVote accounts from the governance program
 * 2. Reads each voter's current veRISE power from their VeLock accounts
 * 3. Tallies votes weighted by veRISE power
 * 4. Falls back to DEFAULT_WEIGHTS if no votes exist
 * 5. Calls set_gauge_weight on the rewards program for each gauge
 *
 * Run once per epoch (after voting period ends):
 *   node tally_and_set_weights.mjs
 */

import pkg from "@coral-xyz/anchor";
const { AnchorProvider, Program, Wallet } = pkg;
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "fs";
import { homedir } from "os";
import path from "path";

const RPC              = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const REWARDS_PROG_ID  = new PublicKey("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8");
const GOV_PROG_ID      = new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");

// Default weights used when no gauge votes have been cast (bps, must sum to 10000)
const DEFAULT_WEIGHTS = [
  { label: "riseSOL/SOL  (Orca)",    pool: makePlaceholder("rise-pool-risesol-sol"),  weightBps: 4000 },
  { label: "riseSOL/USDC (Orca)",    pool: makePlaceholder("rise-pool-risesol-usdc"), weightBps: 3000 },
  { label: "RISE/SOL     (Raydium)", pool: makePlaceholder("rise-pool-rise-sol"),      weightBps: 3000 },
];

function makePlaceholder(seed) {
  return Keypair.fromSeed(Uint8Array.from(Buffer.from(seed.padEnd(32, "\0")))).publicKey;
}

// ─────────────────────────────────────────────────────────────────────────────

const keyPath = path.join(homedir(), ".config/solana/id.json");
const keypair = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(keyPath, "utf8"))));

const rewardsIdl = JSON.parse(readFileSync(new URL("./target/idl/rise_rewards.json",    import.meta.url).pathname, "utf8"));
const govIdl     = JSON.parse(readFileSync(new URL("./target/idl/rise_governance.json", import.meta.url).pathname, "utf8"));

const connection = new Connection(RPC, "confirmed");
const provider   = new AnchorProvider(connection, new Wallet(keypair), { commitment: "confirmed" });
const rewards    = new Program(rewardsIdl, provider);
const gov        = new Program(govIdl, provider);

const [rewardsConfig] = PublicKey.findProgramAddressSync([Buffer.from("rewards_config")], REWARDS_PROG_ID);
const currentSlot = await connection.getSlot();

// ── Step 1: Fetch all GaugeVote accounts ─────────────────────────────────────
console.log("Fetching gauge votes...");
const allGaugeVotes = await gov.account["gaugeVote"].all();
console.log(`  Found ${allGaugeVotes.length} gauge vote(s)`);

// ── Step 2: Fetch veRISE power for each voter ─────────────────────────────────
// VeLock accounts: memcmp by owner at offset 8
const verisePowerByPool = new Map(); // pool base58 → total weighted bps

let totalVerise = 0n;
const voteEntries = [];

for (const gv of allGaugeVotes) {
  const owner = gv.account.owner;
  // Fetch all VeLocks for this owner
  const locks = await gov.account["veLock"].all([
    { memcmp: { offset: 8, bytes: owner.toBase58() } },
  ]);

  // Sum current veRISE across all active locks
  let voterVerise = 0n;
  for (const lock of locks) {
    const acc = lock.account;
    const endSlot = acc.lockEndSlot.toNumber();
    const startSlot = acc.lockStartSlot.toNumber();
    const veriseAmount = BigInt(acc.veriseAmount.toString());
    if (currentSlot < endSlot && endSlot > startSlot) {
      const remaining = BigInt(endSlot - currentSlot);
      const total = BigInt(endSlot - startSlot);
      voterVerise += veriseAmount * remaining / total;
    }
  }

  if (voterVerise === 0n) continue; // expired or no locks

  totalVerise += voterVerise;
  voteEntries.push({ gauges: gv.account.gauges, verise: voterVerise });
}

// ── Step 3: Tally votes weighted by veRISE ────────────────────────────────────
let finalWeights;

if (totalVerise === 0n || voteEntries.length === 0) {
  console.log("No active votes found — using default weights.");
  finalWeights = DEFAULT_WEIGHTS.map(({ label, pool, weightBps }) => ({ label, pool, weightBps }));
} else {
  console.log(`Tallying votes from ${voteEntries.length} voter(s), total veRISE: ${totalVerise}`);

  // Accumulate weighted bps per pool
  const poolTotals = new Map(); // pool base58 → weighted sum

  for (const { gauges, verise } of voteEntries) {
    for (const alloc of gauges) {
      if (alloc.weightBps === 0) continue;
      const poolStr = alloc.pool.toBase58();
      const current = poolTotals.get(poolStr) ?? 0n;
      poolTotals.set(poolStr, current + BigInt(alloc.weightBps) * verise);
    }
  }

  // Convert weighted sums to bps (divide by totalVerise)
  const rawBps = [];
  for (const [poolStr, weightedSum] of poolTotals.entries()) {
    rawBps.push({ pool: new PublicKey(poolStr), bps: Number(weightedSum * 10_000n / totalVerise / 10_000n) });
  }

  // Find matching gauge labels from defaults
  const labelByPool = Object.fromEntries(DEFAULT_WEIGHTS.map(d => [d.pool.toBase58(), d.label]));

  // Normalize to sum exactly to 10,000 — last entry absorbs rounding
  let bpsSum = 0;
  finalWeights = rawBps.map(({ pool, bps }, i) => {
    const adjusted = i === rawBps.length - 1 ? 10_000 - bpsSum : bps;
    bpsSum += adjusted;
    return { label: labelByPool[pool.toBase58()] ?? pool.toBase58().slice(0, 8), pool, weightBps: adjusted };
  });

  // Check if any known gauge pools received votes
  const knownPoolStrs = new Set(DEFAULT_WEIGHTS.map(d => d.pool.toBase58()));
  const hasKnownVotes = finalWeights.some(f => knownPoolStrs.has(f.pool.toBase58()) && f.weightBps > 0);

  if (!hasKnownVotes) {
    console.log("No votes for known gauge pools — falling back to default weights.");
    finalWeights = DEFAULT_WEIGHTS.map(({ label, pool, weightBps }) => ({ label, pool, weightBps }));
  } else {
    // Any default pools not voted on get 0
    for (const def of DEFAULT_WEIGHTS) {
      if (!finalWeights.find(f => f.pool.toBase58() === def.pool.toBase58())) {
        finalWeights.push({ label: def.label, pool: def.pool, weightBps: 0 });
      }
    }
    // Remove unknown pools that don't have on-chain gauges
    finalWeights = finalWeights.filter(f => knownPoolStrs.has(f.pool.toBase58()));
  }
}

// ── Step 4: Apply weights on-chain ────────────────────────────────────────────
console.log("\nApplying weights:");
for (const { label, pool, weightBps } of finalWeights) {
  const [gauge] = PublicKey.findProgramAddressSync(
    [Buffer.from("gauge"), pool.toBuffer()],
    REWARDS_PROG_ID,
  );
  try {
    const tx = await rewards.methods
      .setGaugeWeight(weightBps)
      .accounts({ authority: keypair.publicKey, config: rewardsConfig, gauge })
      .rpc();
    console.log(`  ${label}: ${weightBps} bps (${(weightBps / 100).toFixed(0)}%) — Tx: ${tx.slice(0, 16)}...`);
  } catch (err) {
    console.error(`  Failed for ${label}:`, err.message);
  }
}

console.log("\nDone.");
