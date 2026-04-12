/**
 * Epoch cranks — called once per epoch:
 *   - update_exchange_rate  (staking)
 *   - collect_fees          (staking)
 *   - collect_cdp_fees      (cdp)
 *   - checkpoint_gauge      (rewards, once per active gauge)
 */
import * as anchor from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { RiseClient, PDAS, PROGRAM_IDS, withRetry } from "../client";
import { makeLogger } from "../logger";

const log = makeLogger("epoch");

// ── update_exchange_rate ──────────────────────────────────────────────────────

export async function updateExchangeRate(client: RiseClient): Promise<void> {
  const pool = await (client.staking.account as any).globalPool.fetch(PDAS.globalPool);
  const clock = await client.connection.getEpochInfo();
  const currentEpoch = BigInt(clock.epoch);
  const lastEpoch    = BigInt((pool as any).lastRateUpdateEpoch.toString());

  if (currentEpoch <= lastEpoch) {
    log.info("update_exchange_rate: already updated this epoch", { currentEpoch: currentEpoch.toString(), lastEpoch: lastEpoch.toString() });
    return;
  }

  log.info("update_exchange_rate: calling crank", { currentEpoch: currentEpoch.toString() });

  await withRetry(async () => {
    // stakeLamportsTotal = 0 until validator delegation is built
    await client.staking.methods
      .updateExchangeRate(new anchor.BN(0))
      .accounts({
        caller:    client.wallet.publicKey,
        pool:      PDAS.globalPool,
        poolVault: PDAS.poolVault,
      })
      .rpc({ commitment: "confirmed" });
  }, "update_exchange_rate");

  log.info("update_exchange_rate: done");
}

// ── collect_fees (staking) ────────────────────────────────────────────────────

export async function collectFees(client: RiseClient): Promise<void> {
  const treasury = await (client.staking.account as any).protocolTreasury.fetch(PDAS.treasury);
  const clock     = await client.connection.getEpochInfo();
  const currentEpoch = BigInt(clock.epoch);
  const lastEpoch    = BigInt((treasury as any).lastCollectionEpoch.toString());

  if (currentEpoch <= lastEpoch) {
    log.info("collect_fees: already collected this epoch", { currentEpoch: currentEpoch.toString() });
    return;
  }

  const teamWallet: PublicKey = (treasury as any).teamWallet;
  log.info("collect_fees: calling crank", { currentEpoch: currentEpoch.toString(), teamWallet: teamWallet.toBase58() });

  await withRetry(async () => {
    await client.staking.methods
      .collectFees()
      .accounts({
        caller:        client.wallet.publicKey,
        pool:          PDAS.globalPool,
        treasury:      PDAS.treasury,
        poolVault:     PDAS.poolVault,
        reserveVault:  PDAS.reserveVault,
        veriseVault:   PDAS.veriseVault,
        teamWallet,
        systemProgram: SystemProgram.programId,
      })
      .rpc({ commitment: "confirmed" });
  }, "collect_fees");

  log.info("collect_fees: done");
}

// ── collect_cdp_fees ──────────────────────────────────────────────────────────

export async function collectCdpFees(client: RiseClient): Promise<void> {
  const feeVaultBalance = await client.connection.getBalance(PDAS.cdpFeeVault);
  const minSweepLamports = 1_000_000; // 0.001 SOL — not worth sweeping dust

  if (feeVaultBalance < minSweepLamports) {
    log.info("collect_cdp_fees: fee vault balance too low to sweep", { balanceLamports: feeVaultBalance });
    return;
  }

  log.info("collect_cdp_fees: sweeping fees", { balanceLamports: feeVaultBalance });

  await withRetry(async () => {
    await client.cdp.methods
      .collectCdpFees()
      .accounts({
        caller:         client.wallet.publicKey,
        cdpFeeVault:    PDAS.cdpFeeVault,
        cdpConfig:      PDAS.cdpConfig,
        treasury:       PDAS.treasury,
        reserveVault:   PDAS.reserveVault,
        veriseVault:    PDAS.veriseVault,
        globalPool:     PDAS.globalPool,
        poolVault:      PDAS.poolVault,
        stakingProgram: PROGRAM_IDS.staking,
        systemProgram:  SystemProgram.programId,
      })
      .rpc({ commitment: "confirmed" });
  }, "collect_cdp_fees");

  log.info("collect_cdp_fees: done");
}

// ── checkpoint_gauge (once per active gauge per epoch) ───────────────────────

export async function checkpointAllGauges(client: RiseClient): Promise<void> {
  // Gauge discriminator: sha256("account:Gauge")[..8]
  const GAUGE_DISC = Buffer.from([9, 19, 249, 189, 158, 171, 226, 205]);

  // Fetch all gauge accounts belonging to the rewards program
  const gaugeAccounts = await client.connection.getProgramAccounts(
    PROGRAM_IDS.rewards,
    {
      commitment: "confirmed",
      filters: [
        { memcmp: { offset: 0, bytes: GAUGE_DISC.toString("base64"), encoding: "base64" as const } },
      ],
    }
  );

  if (gaugeAccounts.length === 0) {
    log.info("checkpoint_gauge: no gauges found");
    return;
  }

  // Fetch rewards config to check current epoch
  const rewardsConfig = await (client.rewards.account as any).rewardsConfig.fetch(PDAS.rewardsConfig);
  const currentEpoch  = BigInt((rewardsConfig as any).currentEpoch.toString());

  log.info("checkpoint_gauge: checking gauges", { count: gaugeAccounts.length, currentEpoch: currentEpoch.toString() });

  for (const { pubkey: gaugePda } of gaugeAccounts) {
    let gauge: any;
    try {
      gauge = await (client.rewards.account as any).gauge.fetch(gaugePda);
    } catch {
      // Not a gauge account (wrong discriminator) — skip
      continue;
    }

    if (!gauge.active) {
      log.debug("checkpoint_gauge: gauge inactive, skipping", { gauge: gaugePda.toBase58() });
      continue;
    }

    const lastCheckpoint = BigInt(gauge.lastCheckpointEpoch.toString());
    if (lastCheckpoint >= currentEpoch) {
      log.info("checkpoint_gauge: already checkpointed this epoch", { gauge: gaugePda.toBase58() });
      continue;
    }

    log.info("checkpoint_gauge: checkpointing gauge", { gauge: gaugePda.toBase58(), pool: gauge.pool.toBase58() });

    await withRetry(async () => {
      await client.rewards.methods
        .checkpointGauge()
        .accounts({
          caller: client.wallet.publicKey,
          config: PDAS.rewardsConfig,
          gauge:  gaugePda,
        })
        .rpc({ commitment: "confirmed" });
    }, `checkpoint_gauge(${gaugePda.toBase58().slice(0, 8)})`);

    log.info("checkpoint_gauge: done", { gauge: gaugePda.toBase58() });
  }
}

// ── Run all epoch cranks ──────────────────────────────────────────────────────

export async function runEpochCranks(client: RiseClient): Promise<void> {
  log.info("--- epoch cranks start ---");
  const tasks: Array<[string, () => Promise<void>]> = [
    ["update_exchange_rate", () => updateExchangeRate(client)],
    ["collect_fees",         () => collectFees(client)],
    ["collect_cdp_fees",     () => collectCdpFees(client)],
    ["checkpoint_gauges",    () => checkpointAllGauges(client)],
  ];

  for (const [name, task] of tasks) {
    try {
      await task();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log.error(`${name}: failed after retries`, { error: msg });
    }
  }

  log.info("--- epoch cranks done ---");
}
