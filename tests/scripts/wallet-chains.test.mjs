import {test} from 'node:test';
import assert from 'node:assert/strict';
import {balance, cashaddr, chronikTotal, midnight as replay} from '../../frontend/wallet-chains.js';

// Public test vectors only (BIP-39 "abandon … about", eCash spec vectors).
const net = (chain, endpoint, name = chain) => ({chain, endpoint, name});
const reply = (body, status = 200) => ({ok: status < 400, status, json: async () => body, arrayBuffer: async () => body});
function fetcher(routes) {
  const calls = [];
  return {calls, fetch: async (url, init = {}) => {
    calls.push({url, body: init.body ? JSON.parse(init.body) : undefined, headers: init.headers});
    for (const [match, answer] of routes) if (url.includes(match)) return typeof answer === 'function' ? answer(init) : answer;
    throw new Error(`unexpected ${url}`);
  }};
}

test('Solana: lamports for SOL, and an SPL token from its one associated account, decimals checked', async () => {
  const solana = {...net('solana', 'https://api.devnet.solana.com', 'Solana Devnet'), key: 'solana-devnet'};
  const sol = {standard: 'native', symbol: 'SOL', decimals: 9};
  const usdc = {standard: 'spl-token', symbol: 'USDC', decimals: 6, id: '4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU', account: 'Ata1111111111111111111111111111111111111111'};
  const owner = 'HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk';
  const tokenAccount = ({amount = '3000000', decimals = 6, holder = owner, mint = usdc.id, program = 'spl-token'} = {}) =>
    reply({result: {value: {data: {program, parsed: {type: 'account', info: {mint, owner: holder, tokenAmount: {amount, decimals}}}}}}});
  let io = fetcher([['solana', init => {
    const {method, params} = JSON.parse(init.body);
    if (method === 'getGenesisHash') return reply({result: 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG'});
    if (method === 'getBalance') return reply({result: {value: 1500000000}});
    assert.equal(method, 'getAccountInfo');
    assert.equal(params[0], usdc.account);
    assert.equal(params[1].encoding, 'jsonParsed');
    return tokenAccount();
  }]]);
  assert.equal(await balance(solana, sol, owner, io), '1500000000');
  assert.equal(await balance(solana, usdc, owner, io), '3000000');
  // The rest answer every call alike; drop the key to skip the genesis check.
  const any = {...solana, key: undefined};
  await assert.rejects(balance(any, usdc, owner, fetcher([['solana', tokenAccount({decimals: 9})]])), /9 decimals, not 6/);
  // A node from another cluster is refused before anything is read.
  await assert.rejects(balance(solana, sol, owner, fetcher([['solana', reply({result: '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'})]])), /different cluster/);
  // Never created: nothing held.
  assert.equal(await balance(any, usdc, owner, fetcher([['solana', reply({result: {value: null}})]])), '0');
  // Seen on mainnet: the vector address's account handed to another owner. Not ours.
  assert.equal(await balance(any, usdc, owner, fetcher([['solana', tokenAccount({holder: 'usdc8UkQMkuhJmcceTmrJBcW5P2V5LRZGy9MCGvH6Lw'})]])), '0');
  await assert.rejects(balance(any, usdc, owner, fetcher([['solana', tokenAccount({mint: 'Other'})]])), /not what it should be/);
  await assert.rejects(balance(solana, {...usdc, account: undefined}, owner, io), /No token account/);
  await assert.rejects(balance(any, sol, owner, fetcher([['solana', reply({error: {message: 'rate limited'}})]])), /rate limited/);
});

test('Cardano: lovelace from address_info, assets matched by policy and name and summed', async () => {
  const cardano = {...net('cardano', 'https://preprod.koios.rest/api/v1'), key: 'cardano-preprod'};
  const ada = {standard: 'native'};
  const djed = {standard: 'cardano-asset', id: '8db269c3ec630e06ae29f74bc39edd1f87c819f1056206e879a1cd61446a65644d6963726f555344'};
  const policy = djed.id.slice(0, 56), name = djed.id.slice(56);
  // Through the backend relay: same-origin, one public address, a named query.
  const relay = answers => fetcher([['/api/wallet/cardano', init => reply(answers[JSON.parse(init.body).path])]]);
  const io = relay({
    address_info: [{balance: '12345678'}],
    address_assets: [{policy_id: policy, asset_name: name, quantity: '700'}, {policy_id: policy, asset_name: '00', quantity: '9'}, {policy_id: policy, asset_name: name, quantity: '300'}],
  });
  assert.equal(await balance(cardano, ada, 'addr_test1q', io), '12345678');
  assert.equal(await balance(cardano, djed, 'addr_test1q', io), '1000');
  assert.deepEqual(io.calls[0].body, {network: 'cardano-preprod', path: 'address_info', address: 'addr_test1q'});
  assert.ok(io.calls.every(c => c.url === '/api/wallet/cardano'), 'never Koios directly');
  // A fresh address has no row at all: zero, not an error.
  assert.equal(await balance(cardano, ada, 'x', relay({address_info: []})), '0');
  await assert.rejects(balance(cardano, ada, 'x', relay({address_info: [{balance: '1e9'}]})), /not a number/);
  // The relay's own refusal reaches the page as written.
  await assert.rejects(balance(cardano, ada, 'x', fetcher([['/api/wallet/cardano', reply({error: 'That is not a Cardano Preprod address.'}, 400)]])), /not a Cardano Preprod address/);
});

test('eCash: CashAddr decodes to the spec hash, and Chronik protobuf sums every output', async () => {
  // The derived account-0 vector, on both networks.
  for (const address of ['ecash:qrwzys2q6xq98vwz0kjn6ulu5m6yljr5fyc909kalg', 'ectest:qrwzys2q6xq98vwz0kjn6ulu5m6yljr5fy7w393sue']) {
    assert.deepEqual(cashaddr(address), {type: 'p2pkh', hash: 'dc224140d18053b1c27da53d73fca6f44fc87449'});
  }
  assert.deepEqual(cashaddr('ecash:pquc59839pv8fga4h8eayy5fty0s00aj5czp4d547x'), {type: 'p2sh', hash: '398a14f1285874a3b5b9f3d21289591f07bfb2a6'});
  assert.throws(() => cashaddr('ecash:qrwzys2q6xq98vwz0kjn6ulu5m6yljr5fyc909kalh'), /checksum/);
  // A mainnet body under the testnet prefix fails: the prefix is in the checksum.
  assert.throws(() => cashaddr('ectest:qrwzys2q6xq98vwz0kjn6ulu5m6yljr5fyc909kalg'), /checksum/);

  // ScriptUtxos { 2: Utxo { 5: sats, 11: token } … }, plus an unknown field to skip.
  const varint = n => { const out = []; let v = BigInt(n); do { let b = Number(v & 0x7fn); v >>= 7n; out.push(v ? b | 0x80 : b); } while (v); return out; };
  const field = (number, bytes) => [...varint((number << 3) | 2), ...varint(bytes.length), ...bytes];
  const utxo = sats => [...varint((5 << 3) | 0), ...varint(sats), ...field(11, [1, 2])];
  const message = Uint8Array.from([...field(1, [9, 9]), ...field(2, utxo(54600)), ...field(2, utxo(300000000000))]);
  assert.equal(chronikTotal(message), 300000054600n);
  assert.throws(() => chronikTotal(Uint8Array.from([0x12, 0x10, 0x28])), /truncated/);

  const ecash = net('ecash', 'https://chronik.e.cash', 'eCash Mainnet');
  const io = fetcher([['/script/p2pkh/dc224140d18053b1c27da53d73fca6f44fc87449/utxos', reply(message.buffer)]]);
  assert.equal(await balance(ecash, {standard: 'native'}, 'ecash:qrwzys2q6xq98vwz0kjn6ulu5m6yljr5fyc909kalg', io), '300000054600');
  assert.equal(io.calls[0].headers.accept, 'application/x-protobuf');
  // Chronik answers 404 for an address that never received anything.
  assert.equal(await balance(ecash, {standard: 'native'}, 'ecash:qrwzys2q6xq98vwz0kjn6ulu5m6yljr5fyc909kalg', fetcher([['chronik', reply(null, 404)]])), '0');
});

class FakeSocket {
  static script = [];
  constructor(url, protocol) {
    this.url = url; this.protocol = protocol; this.sent = [];
    FakeSocket.last = this;
    setTimeout(() => this.onopen?.(), 0);
  }
  send(text) {
    const message = JSON.parse(text);
    this.sent.push(message);
    if (message.type === 'connection_init') setTimeout(() => this.onmessage({data: JSON.stringify({type: 'connection_ack'})}), 0);
    if (message.type === 'subscribe') for (const item of FakeSocket.script) setTimeout(() => this.onmessage({data: JSON.stringify({id: '1', type: 'next', payload: {data: {unshieldedTransactions: item}}})}), 0);
  }
  close() { this.closed = true; }
}

test('Midnight: the replay is summed only once it reaches the progress marker', async () => {
  const midnight = net('midnight', 'https://indexer.preview.midnight.network/api/v4/graphql', 'Midnight Preview');
  const me = 'mn_addr_test1me', NIGHT = '0'.repeat(64);
  FakeSocket.script = [
    {__typename: 'UnshieldedTransactionsProgress', highestTransactionId: 7},
    {__typename: 'UnshieldedTransaction', transaction: {id: 3}, createdUtxos: [{owner: me, tokenType: NIGHT, value: '5000000', spentAtTransaction: {id: 6}}, {owner: 'someone', tokenType: NIGHT, value: '9'}]},
    {__typename: 'UnshieldedTransaction', transaction: {id: 7}, createdUtxos: [{owner: me, tokenType: NIGHT, value: '2500000', spentAtTransaction: null}, {owner: me, tokenType: 'ab'.repeat(32), value: '1'}]},
  ];
  const io = {WebSocket: FakeSocket};
  assert.equal(await balance(midnight, {standard: 'native'}, me, io), '2500000');
  const socket = FakeSocket.last;
  assert.equal(socket.url, 'wss://indexer.preview.midnight.network/api/v4/graphql/ws');
  assert.equal(socket.protocol, 'graphql-transport-ws');
  assert.deepEqual(socket.sent.find(m => m.type === 'subscribe').payload.variables, {addr: me});
  assert.ok(socket.sent.some(m => m.type === 'complete') && socket.closed);
  // A replay that never reaches the marker is refused, never summed.
  FakeSocket.script = [{__typename: 'UnshieldedTransactionsProgress', highestTransactionId: 9}, {__typename: 'UnshieldedTransaction', transaction: {id: 3}, createdUtxos: [{owner: me, value: '1'}]}];
  await assert.rejects(replay(io, midnight, {standard: 'native'}, me, 200), /did not finish/);
});

test('an EVM or unknown chain is not this module\'s job', async () => {
  await assert.rejects(balance({chain: 'evm', name: 'Cronos'}, {}, '0x'), /No balance reader/);
});
