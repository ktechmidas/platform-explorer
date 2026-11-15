const assert = require('node:assert/strict')
const http = require('node:http')
const fixture = require('./fixtures/palmatum-core24-block.json')
const fixed = process.env.EXPECT_FIXED === '1'
const calls = []
let fail = false
const server = http.createServer((req, res) => {
  let bytes = ''
  req.on('data', chunk => { bytes += chunk })
  req.on('end', () => {
    const q = JSON.parse(bytes)
    calls.push(q)
    assert.equal(req.headers.authorization, 'Basic ' + Buffer.from('test:test').toString('base64'))
    assert.equal(q.method, 'quorum')
    const typed = q.params.length === 1 || Number.isInteger(q.params[1])
    res.statusCode = typed && !fail ? 200 : 500
    res.setHeader('Content-Type', 'application/json')
    res.end(JSON.stringify({ id: q.id, result: typed && !fail ? { fixture: true } : null,
      error: typed && !fail ? null : { code: -3, message: 'Position 1 (height): JSON value of type string is not of expected type number' } }))
  })
})
server.listen(0, '127.0.0.1', async () => {
  try {
    Object.assign(process.env, { DASHCORE_HOST: '127.0.0.1', DASHCORE_PORT: String(server.address().port), DASHCORE_USER: 'test', DASHCORE_PASS: 'test' })
    const C = require('/app/src/dashcoreRpc.js')
    const height = fixture.header.l1LockedHeight
    if (fixed) assert.equal((await C.getQuorumsListExtended(height)).fixture, true)
    else await assert.rejects(C.getQuorumsListExtended(height))
    assert.deepEqual(calls.at(-1).params, ['listextended', fixed ? height : String(height)])
    assert.equal((await C.getQuorumsListExtended()).fixture, true)
    assert.deepEqual(calls.at(-1).params, ['listextended'])
    // Preserve the already deployed numeric-info adapter and its defaults.
    for (const type of ['107', 107, undefined]) {
      assert.equal((await C.getQuorumInfo(fixture.header.quorumHash, type)).fixture, true)
      assert.deepEqual(calls.at(-1).params, ['info', type === undefined ? 1 : 107, fixture.header.quorumHash])
    }
    if (fixed) {
      assert.equal((await C.getQuorumsListExtended(0)).fixture, true)
      assert.deepEqual(calls.at(-1).params, ['listextended', 0])
      const count = calls.length
      for (const invalid of [-1, 1.5, NaN, true, '', '15288', Number.MAX_SAFE_INTEGER + 1]) await assert.rejects(C.getQuorumsListExtended(invalid))
      assert.equal(calls.length, count)
      fail = true
      await assert.rejects(C.getQuorumsListExtended(height))
      await assert.rejects(C.getQuorumInfo(fixture.header.quorumHash, 107))
    }
    console.log(JSON.stringify({ fixed, reproducedHeight: height, passed: true, requests: calls.length }))
    server.closeAllConnections(); server.close(() => process.exit(0))
  } catch (error) {
    console.error(error.message)
    server.closeAllConnections(); server.close(() => process.exit(1))
  }
})
