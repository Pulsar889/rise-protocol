/**
 * One-off devnet cleanup: closes old-format CdpPosition accounts (163 bytes)
 * that can no longer be deserialized by the current program.
 *
 * Run with: node close_stale_positions.mjs [--exec]
 * Without --exec it dry-runs and prints what it would do.
 */
import * as anchor from "@coral-xyz/anchor";
import { PublicKey, Connection, Keypair } from "@solana/web3.js";
import { readFileSync } from "fs";
import { homedir } from "os";

const EXEC = process.argv.includes("--exec");
const CDP_PROGRAM_ID = new PublicKey("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");
const STALE_POSITIONS = [
  // nonce 0 and 1 for FMuzecVsCM2QepJet1PGh7jA35xi35PTiMoEbJw64N34
  "DUepv8WpTdMWrunSjaPkMth169Y4RK72kN2RZL7nTLPK",
  "6Lp2YCoJThqqRPdQyQGwNFETRVErYRqcLpEbKMej12Bt",
];

const keypairPath = `${homedir()}/.config/solana/id.json`;
const kp = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(keypairPath))));
const connection = new Connection("https://api.devnet.solana.com", "confirmed");
const wallet = new anchor.Wallet(kp);
const provider = new anchor.AnchorProvider(connection, wallet, { commitment: "confirmed" });

const idl = JSON.parse(readFileSync("target/idl/rise_cdp.json"));
const program = new anchor.Program(idl, provider);

function derivePda(seeds) {
  return PublicKey.findProgramAddressSync(seeds, CDP_PROGRAM_ID);
}

async function main() {
  const [cdpConfig] = derivePda([Buffer.from("cdp_config")]);
  console.log("Authority:", kp.publicKey.toBase58());
  console.log("CDP config:", cdpConfig.toBase58());

  for (const posStr of STALE_POSITIONS) {
    const stalePosition = new PublicKey(posStr);
    const info = await connection.getAccountInfo(stalePosition);
    if (!info) {
      console.log(`\nSkipping ${posStr} — account not found (already closed?)`);
      continue;
    }

    // Read owner (bytes 8..40) and nonce (byte 144) from raw data.
    const data = info.data;
    const owner = new PublicKey(data.slice(8, 40));
    const nonce = data[144];
    console.log(`\nStale position: ${posStr}`);
    console.log(`  owner: ${owner.toBase58()}, nonce: ${nonce}, size: ${data.length}B`);

    // Derive borrow_rewards PDA from the stale position key.
    const [borrowRewards] = derivePda([Buffer.from("borrow_rewards"), stalePosition.toBuffer()]);
    const brInfo = await connection.getAccountInfo(borrowRewards);
    console.log(`  borrow_rewards: ${borrowRewards.toBase58()} — ${brInfo ? brInfo.data.length + "B" : "missing"}`);

    if (!EXEC) {
      console.log("  DRY RUN — pass --exec to close");
      continue;
    }

    try {
      const tx = await program.methods
        .closeStalePosition()
        .accounts({
          authority:     kp.publicKey,
          cdpConfig,
          stalePosition,
          borrowRewards,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc({ commitment: "confirmed" });
      console.log(`  Closed! tx: ${tx}`);
    } catch (e) {
      console.error(`  Error closing ${posStr}:`, e.message ?? e);
    }
  }

  console.log("\nDone.");
}

main().catch(console.error);
