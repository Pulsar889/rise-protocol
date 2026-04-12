"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.runLiquidationMonitor = runLiquidationMonitor;
/**
 * Liquidation monitor — polls all open CDP positions and liquidates any
 * with a health factor below the configured threshold.
 *
 * Flow for each unhealthy position:
 *   1. accrue_interest  — bring interest current before computing HF
 *   2. getJupiterRoute  — quote collateral → WSOL swap
 *   3. liquidate        — execute via CDP program CPI to Jupiter
 */
const web3_js_1 = require("@solana/web3.js");
const spl_token_1 = require("@solana/spl-token");
const client_1 = require("../client");
const logger_1 = require("../logger");
const log = (0, logger_1.makeLogger)("liquidator");
// ── Constants ─────────────────────────────────────────────────────────────────
// health_factor is u128 scaled by 1e18. Values < RATE_SCALE are liquidatable.
const RATE_SCALE = BigInt("1000000000000000000"); // 1e18
const JUPITER_PROGRAM_ID = new web3_js_1.PublicKey("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
const JUPITER_PROGRAM_AUTHORITY = new web3_js_1.PublicKey("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1");
const WSOL_MINT = new web3_js_1.PublicKey("So11111111111111111111111111111111111111112");
const JUPITER_EVENT_AUTHORITY = web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("__event_authority")], JUPITER_PROGRAM_ID)[0];
// CdpPosition account discriminator bytes (sha256("account:CdpPosition")[..8])
const CDP_POSITION_DISC = Buffer.from([64, 254, 135, 230, 41, 129, 38, 9]);
// Byte offset of the `is_open: bool` field within CdpPosition
// disc(8) + owner(32) + collateralMint(32) + collateralAmountOriginal(8) +
// collateralUsdValue(16) + riseSolDebtPrincipal(8) + interestAccrued(8) +
// lastAccrualSlot(8) + healthFactor(16) + openedAtSlot(8) + nonce(1) = 145
const IS_OPEN_OFFSET = 145;
// health_factor field offset: disc(8)+owner(32)+collMint(32)+collAmt(8)+collUsd(16)+debt(8)+interest(8)+lastSlot(8) = 120
const HEALTH_FACTOR_OFFSET = 120;
// collateralMint field offset: disc(8) + owner(32) = 40
const COLLATERAL_MINT_OFFSET = 40;
// owner field offset: disc(8)
const OWNER_OFFSET = 8;
// ── Helpers ───────────────────────────────────────────────────────────────────
function deriveCdpWsolVault() {
    return web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("cdp_wsol_vault")], client_1.PROGRAM_IDS.cdp)[0];
}
function deriveCollateralConfig(mint) {
    return web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("collateral_config"), mint.toBuffer()], client_1.PROGRAM_IDS.cdp)[0];
}
function deriveCollateralVault(mint) {
    return web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("collateral_vault"), mint.toBuffer()], client_1.PROGRAM_IDS.cdp)[0];
}
function deriveBorrowRewards(positionPubkey) {
    return web3_js_1.PublicKey.findProgramAddressSync([Buffer.from("borrow_rewards"), positionPubkey.toBuffer()], client_1.PROGRAM_IDS.cdp)[0];
}
async function getJupiterRoute(inputMint, outputMint, amountLamports, slippageBps) {
    try {
        const quoteRes = await fetch(`https://quote-api.jup.ag/v6/quote?inputMint=${inputMint}&outputMint=${outputMint}` +
            `&amount=${amountLamports}&slippageBps=${slippageBps}`);
        if (!quoteRes.ok) {
            log.warn("Jupiter quote API failed", { status: quoteRes.status });
            return null;
        }
        const quote = await quoteRes.json();
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
        const swapData = await swapRes.json();
        // Extract routePlanData from serialized instruction:
        // [8 disc][1 id][N route_plan_data][8 in_amount][8 quoted_out][2 slippage][1 fee]
        const rawData = Buffer.from(swapData.swapInstruction.data, "base64");
        const routePlanData = rawData.slice(9, rawData.length - 19);
        const quotedOutAmount = Number(quote.outAmount);
        const accounts = swapData.swapInstruction.accounts;
        const jupiterSourceToken = new web3_js_1.PublicKey(accounts[4].pubkey);
        const jupiterDestinationToken = new web3_js_1.PublicKey(accounts[5].pubkey);
        // Remaining accounts are route-step-specific accounts starting at index 11
        const remainingAccounts = accounts.slice(11).map(a => ({
            pubkey: new web3_js_1.PublicKey(a.pubkey),
            isWritable: a.isWritable,
            isSigner: a.isSigner,
        }));
        return { routePlanData, quotedOutAmount, jupiterSourceToken, jupiterDestinationToken, remainingAccounts };
    }
    catch (err) {
        log.warn("Jupiter route fetch threw", { error: err instanceof Error ? err.message : String(err) });
        return null;
    }
}
// ── accrue_interest ───────────────────────────────────────────────────────────
async function accrueInterest(client, positionPubkey, collateralMint) {
    const collateralConfig = deriveCollateralConfig(collateralMint);
    await (0, client_1.withRetry)(async () => {
        await client.cdp.methods
            .accrueInterest()
            .accounts({
            caller: client.wallet.publicKey,
            position: positionPubkey,
            collateralConfig,
            cdpConfig: client_1.PDAS.cdpConfig,
            globalPool: client_1.PDAS.globalPool,
        })
            .rpc({ commitment: "confirmed" });
    }, `accrue_interest(${positionPubkey.toBase58().slice(0, 8)})`);
}
// ── liquidate ─────────────────────────────────────────────────────────────────
async function liquidatePosition(client, positionPubkey, owner, collateralMint) {
    const slippageBps = Number(process.env.LIQUIDATION_SLIPPAGE_BPS ?? 100);
    const collateralConfig = deriveCollateralConfig(collateralMint);
    const collateralVault = deriveCollateralVault(collateralMint);
    const borrowRewards = deriveBorrowRewards(positionPubkey);
    const cdpWsolVault = deriveCdpWsolVault();
    // Fetch collateral amount from position to size the Jupiter quote
    const position = await client.cdp.account.cdpPosition.fetch(positionPubkey);
    const collateralAmount = Number(position.collateralAmountOriginal.toString());
    if (collateralAmount === 0) {
        log.warn("liquidate: position has zero collateral, skipping", { position: positionPubkey.toBase58() });
        return;
    }
    // Fetch pythPriceFeed from collateralConfig and solPriceFeed from solPaymentConfig
    const [colConfig, solPaymentConfigData] = await Promise.all([
        client.cdp.account.collateralConfig.fetch(collateralConfig),
        client.cdp.account.paymentConfig.fetch(client_1.PDAS.solPaymentConfig),
    ]);
    const pythPriceFeed = colConfig.pythPriceFeed;
    const solPriceFeed = solPaymentConfigData.pythPriceFeed;
    // Get Jupiter route: collateral → WSOL
    const route = await getJupiterRoute(collateralMint.toBase58(), WSOL_MINT.toBase58(), collateralAmount, slippageBps);
    if (!route) {
        log.warn("liquidate: could not get Jupiter route, skipping", {
            position: positionPubkey.toBase58(),
            collateralMint: collateralMint.toBase58(),
        });
        return;
    }
    // Ensure caller has an ATA for the collateral token (to receive trigger fee)
    const callerCollateralAccount = await (0, spl_token_1.getOrCreateAssociatedTokenAccount)(client.connection, client.wallet.payer, collateralMint, client.wallet.publicKey);
    // Borrower's collateral ATA (receives excess collateral; may or may not exist)
    const borrowerCollateralAccount = (0, spl_token_1.getAssociatedTokenAddressSync)(collateralMint, owner);
    log.info("liquidate: executing", {
        position: positionPubkey.toBase58(),
        collateralMint: collateralMint.toBase58(),
        collateralAmount,
        quotedOutAmount: route.quotedOutAmount,
    });
    await (0, client_1.withRetry)(async () => {
        await client.cdp.methods
            .liquidate(Buffer.from(route.routePlanData), route.quotedOutAmount, slippageBps)
            .accounts({
            caller: client.wallet.publicKey,
            position: positionPubkey,
            collateralConfig,
            collateralMint,
            collateralVault,
            callerCollateralAccount: callerCollateralAccount.address,
            borrowerCollateralAccount,
            globalPool: client_1.PDAS.globalPool,
            cdpConfig: client_1.PDAS.cdpConfig,
            cdpFeeVault: client_1.PDAS.cdpFeeVault,
            poolVault: client_1.PDAS.poolVault,
            wsolMint: WSOL_MINT,
            cdpWsolVault,
            solPaymentConfig: client_1.PDAS.solPaymentConfig,
            pythPriceFeed,
            solPriceFeed,
            tokenProgram: spl_token_1.TOKEN_PROGRAM_ID,
            systemProgram: web3_js_1.SystemProgram.programId,
            jupiterProgram: JUPITER_PROGRAM_ID,
            jupiterProgramAuthority: JUPITER_PROGRAM_AUTHORITY,
            jupiterEventAuthority: JUPITER_EVENT_AUTHORITY,
            jupiterSourceToken: route.jupiterSourceToken,
            jupiterDestinationToken: route.jupiterDestinationToken,
            borrowRewardsConfig: client_1.PDAS.borrowRewardsConfig,
            borrowRewards,
        })
            .remainingAccounts(route.remainingAccounts)
            .rpc({ commitment: "confirmed" });
    }, `liquidate(${positionPubkey.toBase58().slice(0, 8)})`);
    log.info("liquidate: position liquidated", { position: positionPubkey.toBase58() });
}
// ── Main monitor loop ─────────────────────────────────────────────────────────
async function runLiquidationMonitor(client) {
    const hfThreshold = BigInt(process.env.LIQUIDATION_HF_THRESHOLD ?? "990000000000000000"); // 0.99 * 1e18
    log.info("liquidation monitor: scanning positions", { hfThreshold: hfThreshold.toString() });
    // Fetch all open CDP positions via getProgramAccounts
    // Filter 1: discriminator matches CdpPosition
    // Filter 2: is_open == true (byte 145 == 1)
    let positionAccounts;
    try {
        positionAccounts = await client.connection.getProgramAccounts(client_1.PROGRAM_IDS.cdp, {
            commitment: "confirmed",
            filters: [
                { memcmp: { offset: 0, bytes: CDP_POSITION_DISC.toString("base64"), encoding: "base64" } },
                { memcmp: { offset: IS_OPEN_OFFSET, bytes: Buffer.from([1]).toString("base64"), encoding: "base64" } },
            ],
        });
    }
    catch (err) {
        log.error("liquidation monitor: getProgramAccounts failed", { error: err instanceof Error ? err.message : String(err) });
        return;
    }
    log.info("liquidation monitor: found open positions", { count: positionAccounts.length });
    let checked = 0, liquidated = 0, skipped = 0;
    for (const { pubkey: positionPubkey, account } of positionAccounts) {
        const data = account.data;
        // Read health_factor (u128 little-endian, 16 bytes) at offset 120
        const hfBytes = data.slice(HEALTH_FACTOR_OFFSET, HEALTH_FACTOR_OFFSET + 16);
        const healthFactor = hfBytes.readBigUInt64LE(0) + (hfBytes.readBigUInt64LE(8) << BigInt(64));
        checked++;
        if (healthFactor >= hfThreshold) {
            log.debug("liquidation monitor: position healthy", {
                position: positionPubkey.toBase58(),
                healthFactor: (Number(healthFactor) / Number(RATE_SCALE)).toFixed(4),
            });
            continue;
        }
        // Read owner (pubkey, 32 bytes at offset 8)
        const owner = new web3_js_1.PublicKey(data.slice(OWNER_OFFSET, OWNER_OFFSET + 32));
        const collateralMint = new web3_js_1.PublicKey(data.slice(COLLATERAL_MINT_OFFSET, COLLATERAL_MINT_OFFSET + 32));
        log.warn("liquidation monitor: UNHEALTHY position found", {
            position: positionPubkey.toBase58(),
            owner: owner.toBase58(),
            collateralMint: collateralMint.toBase58(),
            healthFactor: (Number(healthFactor) / Number(RATE_SCALE)).toFixed(4),
        });
        try {
            // Step 1: accrue interest to bring position current
            await accrueInterest(client, positionPubkey, collateralMint);
            // Re-fetch after accrual — position may now be healthy (edge case)
            const refreshed = await client.cdp.account.cdpPosition.fetch(positionPubkey);
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
        }
        catch (err) {
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
//# sourceMappingURL=liquidator.js.map