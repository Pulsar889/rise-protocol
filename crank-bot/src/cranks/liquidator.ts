/**
 * Liquidation monitor — polls all open CDP positions and liquidates any
 * with a health factor below the configured threshold.
 *
 * Flow for each unhealthy position:
 *   1. accrue_interest  — bring interest current before computing HF
 *   2. getJupiterRoute  — quote collateral → WSOL swap
 *   3. liquidate        — execute via CDP program CPI to Jupiter
 */
import { PublicKey, SystemProgram, AccountMeta } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import { RiseClient, PDAS, PROGRAM_IDS, withRetry } from "../client";
import { makeLogger } from "../logger";

const log = makeLogger("liquidator");

// ── Constants ─────────────────────────────────────────────────────────────────

// health_factor is u128 scaled by 1e18. Values < RATE_SCALE are liquidatable.
const RATE_SCALE = BigInt("1000000000000000000"); // 1e18

const JUPITER_PROGRAM_ID        = new PublicKey("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
const JUPITER_PROGRAM_AUTHORITY = new PublicKey("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1");
const WSOL_MINT                 = new PublicKey("So11111111111111111111111111111111111111112");

const JUPITER_EVENT_AUTHORITY = PublicKey.findProgramAddressSync(
  [Buffer.from("__event_authority")],
  JUPITER_PROGRAM_ID
)[0];

// CdpPosition account discriminator bytes (sha256("account:CdpPosition")[..8])
const CDP_POSITION_DISC = Buffer.from([64, 254, 135, 230, 41, 129, 38, 9]);

// Byte offset of the `is_open: bool` field within CdpPosition — used only for
// the getProgramAccounts memcmp filter to pre-select open positions server-side.
// disc(8) + owner(32) + collateralMint(32) + collateralAmountOriginal(8) +
// collateralUsdValue(16) + riseSolDebtPrincipal(8) + interestAccrued(8) +
// lastAccrualSlot(8) + healthFactor(16) + openedAtSlot(8) + nonce(1) = 145
const IS_OPEN_OFFSET = 145;

// ── Helpers ───────────────────────────────────────────────────────────────────

function deriveCdpWsolVault(): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("cdp_wsol_vault")],
    PROGRAM_IDS.cdp
  )[0];
}

function deriveCollateralConfig(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("collateral_config"), mint.toBuffer()],
    PROGRAM_IDS.cdp
  )[0];
}

function deriveCollateralVault(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("collateral_vault"), mint.toBuffer()],
    PROGRAM_IDS.cdp
  )[0];
}

function deriveBorrowRewards(positionPubkey: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("borrow_rewards"), positionPubkey.toBuffer()],
    PROGRAM_IDS.cdp
  )[0];
}

// ── Jupiter quote ─────────────────────────────────────────────────────────────

interface JupiterRoute {
  routePlanData:            Buffer;
  quotedOutAmount:          number;
  jupiterSourceToken:       PublicKey;
  jupiterDestinationToken:  PublicKey;
  remainingAccounts:        AccountMeta[];
}

async function getJupiterRoute(
  inputMint: string,
  outputMint: string,
  amountLamports: number,
  slippageBps: number,
): Promise<JupiterRoute | null> {
  try {
    const quoteRes = await fetch(
      `https://quote-api.jup.ag/v6/quote?inputMint=${inputMint}&outputMint=${outputMint}` +
      `&amount=${amountLamports}&slippageBps=${slippageBps}`
    );
    if (!quoteRes.ok) {
      log.warn("Jupiter quote API failed", { status: quoteRes.status });
      return null;
    }
    const quote = await quoteRes.json() as { outAmount: string | number; [key: string]: unknown };

    const swapRes = await fetch("https://quote-api.jup.ag/v6/swap-instructions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        quoteResponse: quote,
        userPublicKey: JUPITER_PROGRAM_AUTHORITY.toBase58(),
        wrapAndUnwrapSol: false,
      }),
    });
    if (!swapRes.ok) {
      log.warn("Jupiter swap-instructions API failed", { status: swapRes.status });
      return null;
    }
    const swapData = await swapRes.json() as {
      swapInstruction: {
        data: string;
        accounts: Array<{ pubkey: string; isWritable: boolean; isSigner: boolean }>;
      };
    };

    // Extract routePlanData from serialized instruction:
    // [8 disc][1 id][N route_plan_data][8 in_amount][8 quoted_out][2 slippage][1 fee]
    const rawData = Buffer.from(swapData.swapInstruction.data, "base64");
    const routePlanData = rawData.slice(9, rawData.length - 19);
    const quotedOutAmount = Number(quote.outAmount);

    const accounts = swapData.swapInstruction.accounts;
    const jupiterSourceToken      = new PublicKey(accounts[4].pubkey);
    const jupiterDestinationToken = new PublicKey(accounts[5].pubkey);

    // Remaining accounts are route-step-specific accounts starting at index 11
    const remainingAccounts: AccountMeta[] = accounts.slice(11).map(a => ({
      pubkey:     new PublicKey(a.pubkey),
      isWritable: a.isWritable,
      isSigner:   a.isSigner,
    }));

    return { routePlanData, quotedOutAmount, jupiterSourceToken, jupiterDestinationToken, remainingAccounts };
  } catch (err: unknown) {
    log.warn("Jupiter route fetch threw", { error: err instanceof Error ? err.message : String(err) });
    return null;
  }
}

// ── accrue_interest ───────────────────────────────────────────────────────────

async function accrueInterest(
  client: RiseClient,
  positionPubkey: PublicKey,
  collateralMint: PublicKey,
): Promise<void> {
  const collateralConfig = deriveCollateralConfig(collateralMint);

  await withRetry(async () => {
    await client.cdp.methods
      .accrueInterest()
      .accounts({
        caller:          client.wallet.publicKey,
        position:        positionPubkey,
        collateralConfig,
        cdpConfig:       PDAS.cdpConfig,
        globalPool:      PDAS.globalPool,
      })
      .rpc({ commitment: "confirmed" });
  }, `accrue_interest(${positionPubkey.toBase58().slice(0, 8)})`);
}

// ── liquidate ─────────────────────────────────────────────────────────────────

async function liquidatePosition(
  client: RiseClient,
  positionPubkey: PublicKey,
  owner: PublicKey,
  collateralMint: PublicKey,
): Promise<void> {
  const slippageBps      = Number(process.env.LIQUIDATION_SLIPPAGE_BPS ?? 100);
  const collateralConfig = deriveCollateralConfig(collateralMint);
  const collateralVault  = deriveCollateralVault(collateralMint);
  const borrowRewards    = deriveBorrowRewards(positionPubkey);
  const cdpWsolVault     = deriveCdpWsolVault();

  // Fetch collateral amount from position to size the Jupiter quote
  const position = await (client.cdp.account as any).cdpPosition.fetch(positionPubkey) as any;
  const collateralAmount = Number(position.collateralAmountOriginal.toString());

  if (collateralAmount === 0) {
    log.warn("liquidate: position has zero collateral, skipping", { position: positionPubkey.toBase58() });
    return;
  }

  // Fetch pythPriceFeed from collateralConfig and solPriceFeed from solPaymentConfig
  const [colConfig, solPaymentConfigData] = await Promise.all([
    (client.cdp.account as any).collateralConfig.fetch(collateralConfig),
    (client.cdp.account as any).paymentConfig.fetch(PDAS.solPaymentConfig),
  ]);
  const pythPriceFeed: PublicKey = (colConfig as any).pythPriceFeed;
  const solPriceFeed:  PublicKey = (solPaymentConfigData as any).pythPriceFeed;

  // Get Jupiter route: collateral → WSOL
  const route = await getJupiterRoute(
    collateralMint.toBase58(),
    WSOL_MINT.toBase58(),
    collateralAmount,
    slippageBps,
  );

  if (!route) {
    log.warn("liquidate: could not get Jupiter route, skipping", {
      position: positionPubkey.toBase58(),
      collateralMint: collateralMint.toBase58(),
    });
    return;
  }

  // Borrower's collateral ATA (receives excess collateral; may or may not exist)
  const borrowerCollateralAccount = getAssociatedTokenAddressSync(collateralMint, owner);

  log.info("liquidate: executing", {
    position: positionPubkey.toBase58(),
    collateralMint: collateralMint.toBase58(),
    collateralAmount,
    quotedOutAmount: route.quotedOutAmount,
  });

  await withRetry(async () => {
    await client.cdp.methods
      .liquidate(
        Buffer.from(route.routePlanData),
        route.quotedOutAmount,
        slippageBps,
      )
      .accounts({
        caller:                   client.wallet.publicKey,
        position:                 positionPubkey,
        collateralConfig,
        collateralMint,
        collateralVault,
        borrowerCollateralAccount,
        globalPool:               PDAS.globalPool,
        cdpConfig:                PDAS.cdpConfig,
        cdpFeeVault:              PDAS.cdpFeeVault,
        poolVault:                PDAS.poolVault,
        wsolMint:                 WSOL_MINT,
        cdpWsolVault,
        solPaymentConfig:         PDAS.solPaymentConfig,
        pythPriceFeed,
        solPriceFeed,
        tokenProgram:             TOKEN_PROGRAM_ID,
        systemProgram:            SystemProgram.programId,
        jupiterProgram:           JUPITER_PROGRAM_ID,
        jupiterProgramAuthority:  JUPITER_PROGRAM_AUTHORITY,
        jupiterEventAuthority:    JUPITER_EVENT_AUTHORITY,
        jupiterSourceToken:       route.jupiterSourceToken,
        jupiterDestinationToken:  route.jupiterDestinationToken,
        borrowRewardsConfig:      PDAS.borrowRewardsConfig,
        borrowRewards,
      })
      .remainingAccounts(route.remainingAccounts)
      .rpc({ commitment: "confirmed" });
  }, `liquidate(${positionPubkey.toBase58().slice(0, 8)})`);

  log.info("liquidate: position liquidated", { position: positionPubkey.toBase58() });
}

// ── Main monitor loop ─────────────────────────────────────────────────────────

export async function runLiquidationMonitor(client: RiseClient): Promise<void> {
  const hfThreshold = BigInt(process.env.LIQUIDATION_HF_THRESHOLD ?? "990000000000000000"); // 0.99 * 1e18

  log.info("liquidation monitor: scanning positions", { hfThreshold: hfThreshold.toString() });

  // Fetch all open CDP positions via getProgramAccounts
  // Filter 1: discriminator matches CdpPosition
  // Filter 2: is_open == true (byte 145 == 1)
  let positionAccounts: Array<{ pubkey: PublicKey; account: { data: Buffer } }>;
  try {
    positionAccounts = await client.connection.getProgramAccounts(
      PROGRAM_IDS.cdp,
      {
        commitment: "confirmed",
        filters: [
          { memcmp: { offset: 0,             bytes: CDP_POSITION_DISC.toString("base64"), encoding: "base64" } },
          { memcmp: { offset: IS_OPEN_OFFSET, bytes: Buffer.from([1]).toString("base64"),  encoding: "base64" } },
        ],
      }
    ) as unknown as Array<{ pubkey: PublicKey; account: { data: Buffer } }>;
  } catch (err: unknown) {
    log.error("liquidation monitor: getProgramAccounts failed", { error: err instanceof Error ? err.message : String(err) });
    return;
  }

  log.info("liquidation monitor: found open positions", { count: positionAccounts.length });

  let checked = 0, liquidated = 0, skipped = 0;

  for (const { pubkey: positionPubkey, account } of positionAccounts) {
    // Use IDL-based deserialization rather than brittle raw byte-offset reads.
    const decoded = client.cdp.coder.accounts.decode("CdpPosition", account.data as Buffer) as {
      healthFactor: { toString(): string };
      owner: PublicKey;
      collateralMint: PublicKey;
    };

    const healthFactor = BigInt(decoded.healthFactor.toString());

    checked++;

    if (healthFactor >= hfThreshold) {
      log.debug("liquidation monitor: position healthy", {
        position: positionPubkey.toBase58(),
        healthFactor: (Number(healthFactor) / Number(RATE_SCALE)).toFixed(4),
      });
      continue;
    }

    const owner          = decoded.owner;
    const collateralMint = decoded.collateralMint;

    log.warn("liquidation monitor: UNHEALTHY position found", {
      position:      positionPubkey.toBase58(),
      owner:         owner.toBase58(),
      collateralMint: collateralMint.toBase58(),
      healthFactor:  (Number(healthFactor) / Number(RATE_SCALE)).toFixed(4),
    });

    try {
      // Step 1: accrue interest to bring position current
      await accrueInterest(client, positionPubkey, collateralMint);

      // Re-fetch after accrual — position may now be healthy (edge case)
      const refreshed = await (client.cdp.account as any).cdpPosition.fetch(positionPubkey) as any;
      const freshHf = BigInt(refreshed.healthFactor.toString());
      if (freshHf >= RATE_SCALE) {
        log.info("liquidation monitor: position healthy after accrual, skipping", {
          position: positionPubkey.toBase58(),
          healthFactor: (Number(freshHf) / Number(RATE_SCALE)).toFixed(4),
        });
        skipped++;
        continue;
      }

      // Step 2: liquidate
      await liquidatePosition(client, positionPubkey, owner, collateralMint);
      liquidated++;
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log.error("liquidation monitor: failed to liquidate", {
        position: positionPubkey.toBase58(),
        error: msg,
      });
      skipped++;
    }
  }

  log.info("liquidation monitor: scan complete", { checked, liquidated, skipped });
}
