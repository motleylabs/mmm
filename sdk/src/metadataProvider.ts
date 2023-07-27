import { Metaplex } from '@metaplex-foundation/js';
import {
  Creator,
  Metadata,
  TokenStandard,
} from '@metaplex-foundation/mpl-token-metadata';
import { Connection, PublicKey } from '@solana/web3.js';

export class MetadataProviderError extends Error {
  name = 'MetadataProviderError';
  constructor(msg: string) {
    super(msg);
  }
}

export abstract class MetadataProvider {
  abstract load(mint: PublicKey): Promise<void>;
  abstract getCreators(mint: PublicKey): Creator[];
  abstract getTokenStandard(mint: PublicKey): TokenStandard | undefined;
  abstract getRuleset(mint: PublicKey): PublicKey | undefined;
  abstract getLoadedMint(): PublicKey | undefined;

  checkMetadataMint(mint: PublicKey) {
    const loadedMint = this.getLoadedMint();
    if (!loadedMint) {
      throw new MetadataProviderError('no metadata loaded');
    }
    if (!loadedMint.equals(mint)) {
      throw new MetadataProviderError('mint mismatch');
    }
  }
}

export class RpcMetadataProvider extends MetadataProvider {
  private connection: Connection;
  private mpl: Metaplex;
  metadata: Metadata | undefined;

  constructor(conn: Connection) {
    super();
    this.connection = conn;
    this.mpl = new Metaplex(conn);
  }

  async load(mint: PublicKey) {
    const metadataAddress = this.mpl.nfts().pdas().metadata({ mint });
    this.metadata = await Metadata.fromAccountAddress(
      this.connection,
      metadataAddress,
    );
  }

  getCreators(mint: PublicKey): Creator[] {
    this.checkMetadataMint(mint);
    return this.metadata!.data.creators ?? [];
  }

  getTokenStandard(mint: PublicKey): TokenStandard | undefined {
    this.checkMetadataMint(mint);
    return this.metadata!.tokenStandard ?? undefined;
  }

  getRuleset(mint: PublicKey): PublicKey | undefined {
    this.checkMetadataMint(mint);
    return this.metadata!.programmableConfig?.ruleSet ?? undefined;
  }

  getLoadedMint(): PublicKey | undefined {
    return this.metadata?.mint;
  }
}

export function rpcMetadataProviderGenerator(connection: Connection) {
  return new RpcMetadataProvider(connection);
}
