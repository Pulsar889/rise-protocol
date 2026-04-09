/**
 * Rise Protocol Crank Bot
 *
 * Runs four independent loops:
 *   - Epoch cranks       (update_exchange_rate, collect_fees, collect_cdp_fees, checkpoint_gauges)
 *   - Reward cranks      (checkpoint_borrow_rewards, checkpoint_stake_rewards)
 *   - Liquidation monitor (accrue_interest + liquidate for unhealthy positions)
 *   - Governance monitor  (execute_proposal for passed + timelocked proposals)
 *
 * Configuration via environment variables — see .env.example.
 */

import * as dotenv from "dotenv";
dotenv.config();

import { createClient, sleep } from "./client";
import { runEpochCranks }          from "./cranks/epochCranks";
import { runRewardCranks }         from "./cranks/rewardCranks";
import { runLiquidationMonitor }   from "./cranks/liquidator";
import { runGovernanceMonitor }    from "./cranks/governance";
import { makeLogger }              from "./logger";

const log = makeLogger("main");

const EPOCH_CRANK_INTERVAL_MS     = Number(process.env.EPOCH_CRANK_INTERVAL_MS     ?? 5  * 60 * 1000); // 5 min
const REWARD_CRANK_INTERVAL_MS    = Number(process.env.REWARD_CRANK_INTERVAL_MS    ?? 10 * 60 * 1000); // 10 min
const LIQUIDATION_POLL_INTERVAL_MS = Number(process.env.LIQUIDATION_POLL_INTERVAL_MS ?? 30 * 1000);    // 30 sec
const GOVERNANCE_POLL_INTERVAL_MS  = Number(process.env.GOVERNANCE_POLL_INTERVAL_MS  ?? 5  * 60 * 1000); // 5 min

// ── Loop runner ───────────────────────────────────────────────────────────────

/**
 * Runs `fn` immediately, then repeats every `intervalMs`.
 * Errors are caught and logged — the loop continues regardless.
 */
async function runLoop(
  name: string,
  intervalMs: number,
  fn: () => Promise<void>,
): Promise<void> {
  log.info(`loop start: ${name}`, { intervalMs });
  while (true) {
    const start = Date.now();
    try {
      await fn();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log.error(`loop error: ${name}`, { error: msg });
    }
    const elapsed = Date.now() - start;
    const wait = Math.max(0, intervalMs - elapsed);
    await sleep(wait);
  }
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  log.info("rise crank bot starting", {
    epochCrankIntervalMs:     EPOCH_CRANK_INTERVAL_MS,
    rewardCrankIntervalMs:    REWARD_CRANK_INTERVAL_MS,
    liquidationPollIntervalMs: LIQUIDATION_POLL_INTERVAL_MS,
    governancePollIntervalMs:  GOVERNANCE_POLL_INTERVAL_MS,
  });

  const client = createClient();
  log.info("client ready", { wallet: client.wallet.publicKey.toBase58() });

  // Check bot wallet balance
  const balance = await client.connection.getBalance(client.wallet.publicKey);
  log.info("bot wallet balance", { lamports: balance, sol: balance / 1e9 });
  if (balance < 10_000_000) { // 0.01 SOL
    log.warn("bot wallet balance is very low — transactions may fail", { lamports: balance });
  }

  // Start all loops concurrently — each runs independently and indefinitely
  await Promise.all([
    runLoop("epoch_cranks",       EPOCH_CRANK_INTERVAL_MS,      () => runEpochCranks(client)),
    runLoop("reward_cranks",      REWARD_CRANK_INTERVAL_MS,     () => runRewardCranks(client)),
    runLoop("liquidation_monitor", LIQUIDATION_POLL_INTERVAL_MS, () => runLiquidationMonitor(client)),
    runLoop("governance_monitor",  GOVERNANCE_POLL_INTERVAL_MS,  () => runGovernanceMonitor(client)),
  ]);
}

main().catch(err => {
  console.error(JSON.stringify({ ts: new Date().toISOString(), level: "error", module: "main", msg: "fatal error", error: err instanceof Error ? err.message : String(err) }));
  process.exit(1);
});
