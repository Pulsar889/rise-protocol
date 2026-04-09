/**
 * Reward accumulator cranks — called frequently (sub-epoch) for precision:
 *   - checkpoint_borrow_rewards  (cdp)
 *   - checkpoint_stake_rewards   (staking)
 */
import { RiseClient, PDAS, withRetry } from "../client";
import { makeLogger } from "../logger";

const log = makeLogger("rewards");

// ── checkpoint_borrow_rewards ─────────────────────────────────────────────────

export async function checkpointBorrowRewards(client: RiseClient): Promise<void> {
  const config = await (client.cdp.account as any).borrowRewardsConfig.fetch(PDAS.borrowRewardsConfig);
  const slot   = await client.connection.getSlot("confirmed");

  const lastSlot = Number((config as any).lastCheckpointSlot.toString());
  if (slot <= lastSlot) {
    log.debug("checkpoint_borrow_rewards: no new slots since last checkpoint");
    return;
  }

  log.info("checkpoint_borrow_rewards: advancing accumulator", { slotDelta: slot - lastSlot });

  await withRetry(async () => {
    await client.cdp.methods
      .checkpointBorrowRewards()
      .accounts({
        caller:              client.wallet.publicKey,
        borrowRewardsConfig: PDAS.borrowRewardsConfig,
      })
      .rpc({ commitment: "confirmed" });
  }, "checkpoint_borrow_rewards");

  log.info("checkpoint_borrow_rewards: done");
}

// ── checkpoint_stake_rewards ──────────────────────────────────────────────────

export async function checkpointStakeRewards(client: RiseClient): Promise<void> {
  // Only call if the stake_rewards_config account exists (may not be initialized on all deployments)
  const configInfo = await client.connection.getAccountInfo(PDAS.stakeRewardsConfig, "confirmed");
  if (!configInfo) {
    log.debug("checkpoint_stake_rewards: stake_rewards_config not initialized, skipping");
    return;
  }

  const config = await (client.staking.account as any).stakeRewardsConfig.fetch(PDAS.stakeRewardsConfig);
  const slot   = await client.connection.getSlot("confirmed");

  const lastSlot = Number((config as any).lastCheckpointSlot.toString());
  if (slot <= lastSlot) {
    log.debug("checkpoint_stake_rewards: no new slots since last checkpoint");
    return;
  }

  log.info("checkpoint_stake_rewards: advancing accumulator", { slotDelta: slot - lastSlot });

  await withRetry(async () => {
    await client.staking.methods
      .checkpointStakeRewards()
      .accounts({
        caller:             client.wallet.publicKey,
        stakeRewardsConfig: PDAS.stakeRewardsConfig,
      })
      .rpc({ commitment: "confirmed" });
  }, "checkpoint_stake_rewards");

  log.info("checkpoint_stake_rewards: done");
}

// ── Run both reward cranks ────────────────────────────────────────────────────

export async function runRewardCranks(client: RiseClient): Promise<void> {
  log.info("--- reward cranks start ---");

  for (const [name, task] of [
    ["checkpoint_borrow_rewards", () => checkpointBorrowRewards(client)],
    ["checkpoint_stake_rewards",  () => checkpointStakeRewards(client)],
  ] as Array<[string, () => Promise<void>]>) {
    try {
      await task();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log.error(`${name}: failed after retries`, { error: msg });
    }
  }

  log.info("--- reward cranks done ---");
}
