/**
 * Disables riseSOL as an accepted collateral type on devnet.
 * Sets CollateralConfig.active = false for the riseSOL mint.
 * Run with: npx ts-node scripts/disable_risesol_collateral.ts
 */
import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import * as fs from "fs";
import * as path from "path";

const RPC = "https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6";
const CDP_PROGRAM_ID = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");
const RISE_SOL_MINT  = new PublicKey("86bHg3K32cRhnfcYTr3RCgKZme4xSLZzMyzWA8qDswHp");

const KEYPAIR_PATH = process.env.ANCHOR_WALLET ??
  `${process.env.HOME}/.config/solana/id.json`;

async function main() {
  const connection = new Connection(RPC, "confirmed");
  const raw = JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf-8"));
  const payer = Keypair.fromSecretKey(Uint8Array.from(raw));
  const wallet = new Wallet(payer);
  const provider = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  const idlPath = path.join(__dirname, "../target/idl/rise_cdp.json");
  const idl = JSON.parse(fs.readFileSync(idlPath, "utf-8"));
  const program = new Program(idl, provider) as any;

  const [collateralConfig] = PublicKey.findProgramAddressSync(
    [Buffer.from("collateral_config"), RISE_SOL_MINT.toBuffer()],
    CDP_PROGRAM_ID,
  );

  console.log("Authority:         ", payer.publicKey.toBase58());
  console.log("riseSOL mint:      ", RISE_SOL_MINT.toBase58());
  console.log("CollateralConfig:  ", collateralConfig.toBase58());
  console.log("Setting active = false...");

  const sig = await program.methods
    .updateCollateralConfig(
      null, // max_ltv_bps
      null, // liquidation_threshold_bps
      null, // liquidation_penalty_bps
      null, // base_rate_bps
      null, // rate_slope1_bps
      null, // rate_slope2_bps
      null, // optimal_utilization_bps
      null, // conversion_slippage_bps
      false, // active
    )
    .accounts({
      authority:        payer.publicKey,
      collateralConfig,
    })
    .rpc();

  console.log("Done:", sig);
}

main().catch((e) => { console.error(e); process.exit(1); });
