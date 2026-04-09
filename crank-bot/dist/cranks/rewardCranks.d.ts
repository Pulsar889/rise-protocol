/**
 * Reward accumulator cranks — called frequently (sub-epoch) for precision:
 *   - checkpoint_borrow_rewards  (cdp)
 *   - checkpoint_stake_rewards   (staking)
 */
import { RiseClient } from "../client";
export declare function checkpointBorrowRewards(client: RiseClient): Promise<void>;
export declare function checkpointStakeRewards(client: RiseClient): Promise<void>;
export declare function runRewardCranks(client: RiseClient): Promise<void>;
//# sourceMappingURL=rewardCranks.d.ts.map