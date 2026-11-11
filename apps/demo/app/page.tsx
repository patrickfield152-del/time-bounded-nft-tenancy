import {
  defineSite,
  formatUnits,
  readNetwork,
  readSettlementAsset,
  readSlots,
  readSiteTerms,
  readerFor,
  type SlotState,
} from '@stele/sdk';
import { Slot, SlotProvider, formatAmount } from '@stele/react';

/**
 * The demo board, rendered from whatever the environment points at.
 *
 * Accounts come from `NEAR_*` (see `.env.example`) — there is no address in this file. The keys
 * below are this app's page structure, and they are also the on-chain token ids; floors are written
 * in the settlement asset's own units and parsed against the decimals `ft_metadata` reports, so the
 * same file works for a 6-decimal mock USD and for native NEAR.
 *
 * Server-rendered on every request: the first paint carries real owners and prices from
 * `readSlots`, and the client provider takes over for focus refetches. Nothing is cached, because a
 * board whose whole point is that positions change hands should not show a stale owner.
 */
export const dynamic = 'force-dynamic';

const BOARD = [
  { key: 'hero.headline', kind: 'text', floor: '1' },
  { key: 'hero.sub', kind: 'text', floor: '0.25' },
  { key: 'nav.link', kind: 'link', floor: '0.1' },
  { key: 'hero.image', kind: 'image', floor: '2' },
  { key: 'footer.note', kind: 'text', floor: '0.05' },
] as const;

const FALLBACKS: Record<string, string> = {
  'hero.headline': 'Positions on a page, priced by the market.',
  'hero.sub': 'Buy, rent, or take. Content renders only against the on-chain hash.',
  'nav.link': 'Read the contract',
  'hero.image': '',
  'footer.note': 'Rent a position and write it with a key that expires with your term.',
};

export default async function Page() {
  const network = readNetwork();
  const reader = readerFor(network);

  const asset = await readSettlementAsset(reader, network.settlementFt);
  const config = defineSite({
    name: 'stele demo',
    symbol: 'STELE',
    site: network.site,
    settlementFt: network.settlementFt,
    decimals: asset.decimals,
    slots: BOARD.map((slot) => ({ key: slot.key, kind: slot.kind, floor: slot.floor })),
  });

  const [slots, terms] = await Promise.all([
    readSlots(reader, config.site, config.keys),
    readSiteTerms(reader, config.site),
  ]);
  const byKey = new Map<string, SlotState>(slots.map((slot) => [slot.key, slot]));

  return (
    <>
      <header style={{ marginBottom: 32 }}>
        <h1 style={{ fontSize: 28, margin: '0 0 8px' }}>{config.name}</h1>
        <p style={{ margin: 0, color: '#8b949e' }}>
          {config.site} · settles in {asset.symbol} · minimum floor{' '}
          {formatUnits(terms.minSlotFloor, asset.decimals)} {asset.symbol}
          {terms.paused ? ' · paused' : ''}
        </p>
        <p style={{ margin: '12px 0 0', color: '#8b949e' }}>
          Every position below is a NEP-171 token. The price shown is what a purchase costs now: the
          effective floor on a first claim, the take price afterwards.
        </p>
      </header>

      <SlotProvider config={config} rpcUrl={network.rpcUrl} initialSlots={slots}>
        <section style={{ display: 'grid', gap: 12 }}>
          {config.slots.map((definition) => {
            const state = byKey.get(definition.key);
            const owner = state?.owner ?? null;
            return (
              <article
                key={definition.key}
                style={{
                  border: '1px solid #30363d',
                  borderRadius: 8,
                  padding: 16,
                  background: '#11161d',
                }}
              >
                <div style={{ display: 'flex', justifyContent: 'space-between', gap: 16 }}>
                  <code style={{ color: '#79c0ff' }}>{definition.key}</code>
                  <span style={{ color: '#8b949e', fontSize: 13 }}>
                    {definition.kind} · floor {formatAmount(definition.floor, asset)}
                  </span>
                </div>

                <div style={{ margin: '10px 0 14px', fontSize: 15 }}>
                  <Slot id={definition.key} fallback={FALLBACKS[definition.key] ?? definition.key} />
                </div>

                {state === undefined || !state.registered ? (
                  <p style={{ margin: 0, color: '#d29922', fontSize: 13 }}>
                    not registered on this site yet — the owner has to register the key before it
                    can be bought.
                  </p>
                ) : (
                  <p style={{ margin: 0, color: '#8b949e', fontSize: 13 }}>
                    {owner === null ? 'unclaimed' : `owner ${owner}`} · {state.isUnclaimed ? 'claim' : 'take'}{' '}
                    for {formatAmount(state.charged, asset)} · {state.takes} purchase
                    {state.takes === 1 ? '' : 's'}
                    {state.tenant !== null
                      ? ` · rented by ${state.tenant} until ${new Date(
                          Number(state.rentalExpiryNs / 1_000_000n),
                        ).toISOString()}`
                      : ''}
                    {state.isListed
                      ? ` · listed at ${formatAmount(state.ratePerDay, asset)}/day`
                      : ''}
                  </p>
                )}
              </article>
            );
          })}
        </section>
      </SlotProvider>

      <footer style={{ marginTop: 40, color: '#8b949e', fontSize: 13 }}>
        <p style={{ margin: 0 }}>
          A tenant holds a function-call key restricted to <code>edit</code> on this site, funded
          with a gas allowance, and the contract rejects any write past their term. The key is
          deleted when the tenancy ends; the expiry is enforced on chain because a function-call key
          has none of its own.
        </p>
      </footer>
    </>
  );
}
