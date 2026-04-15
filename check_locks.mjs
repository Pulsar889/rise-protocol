import { Connection, PublicKey } from "@solana/web3.js";
import { execSync } from "child_process";

const conn = new Connection("https://devnet.helius-rpc.com/?api-key=48e90e75-929f-420e-8b85-cb6ac585e2e6", "confirmed");
const GOV = new PublicKey("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");
const RISE_MINT = new PublicKey("2TysJ9Tw5WLh7hBLmC6iZp73bm6akogYEushJEf8K49Q");

const wallet = new PublicKey(execSync("solana address").toString().trim());
console.log("Wallet:", wallet.toBase58());

// Check RISE mint decimals
const mintInfo = await conn.getParsedAccountInfo(RISE_MINT);
const decimals = mintInfo.value?.data?.parsed?.info?.decimals ?? "unknown";
console.log("RISE mint decimals:", decimals);

// Check rise_vault balance
const [vaultPda] = PublicKey.findProgramAddressSync([Buffer.from("rise_vault")], GOV);
const vaultInfo = await conn.getTokenAccountBalance(vaultPda).catch(() => null);
console.log("rise_vault balance:", vaultInfo ? vaultInfo.value.uiAmount : "NOT FOUND",
            vaultInfo ? `(${vaultInfo.value.amount} raw)` : "");

// Check VeLock accounts for nonces 0-15
console.log("\nVeLock accounts (nonces 0-15):");
let found = 0;
for (let nonce = 0; nonce <= 15; nonce++) {
  const [lockPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("ve_lock"), wallet.toBuffer(), Buffer.from([nonce])],
    GOV
  );
  const info = await conn.getAccountInfo(lockPda);
  if (info) {
    found++;
    const d = info.data;
    // VeLock layout (8 disc + fields):
    // [8,40)   owner: Pubkey
    // [40,48)  rise_locked: u64
    // [48,56)  verise_amount: u64
    // [56,64)  lock_start_slot: u64
    // [64,72)  lock_end_slot: u64
    // [72,88)  last_revenue_index: u128
    // [88,96)  total_revenue_claimed: u64
    // [96,128) nft_mint: Pubkey
    // [128,136) lock_number: u64
    // [136]    nonce: u8
    // [137]    bump: u8
    const riseLocked       = d.readBigUInt64LE(40);
    const veriseAmount     = d.readBigUInt64LE(48);
    const lockStartSlot    = d.readBigUInt64LE(56);
    const lockEndSlot      = d.readBigUInt64LE(64);
    const revenueClaimed   = d.readBigUInt64LE(88);
    const nftMint          = new PublicKey(d.slice(96, 128)).toBase58();
    const lockNumber       = d.readBigUInt64LE(128);
    const storedNonce      = d[136];

    const scale = 10 ** decimals;
    console.log(`  nonce ${nonce}: PDA ${lockPda.toBase58()}`);
    console.log(`    rise_locked:       ${Number(riseLocked)} raw  =  ${Number(riseLocked) / scale} RISE`);
    console.log(`    verise_amount:     ${Number(veriseAmount) / scale} veRISE`);
    console.log(`    lock_start_slot:   ${lockStartSlot}`);
    console.log(`    lock_end_slot:     ${lockEndSlot}`);
    console.log(`    lock_number:       ${lockNumber}`);
    console.log(`    nft_mint:          ${nftMint}`);
    console.log(`    stored nonce:      ${storedNonce}`);
  }
}
if (found === 0) console.log("  None found.");

console.log(`\nTotal VeLocks found: ${found}`);
