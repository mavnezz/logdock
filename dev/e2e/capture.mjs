// Passes every request on to Logdock and keeps a copy of what was sent:
// dev/captured/<n>-<kinds>.envelope (decompressed) and a .json with the
// request's method, path and headers. Real SDK payloads, for the parser's
// fixtures (backend/crates/logdock-sentry/tests/fixtures).

import { mkdirSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { join } from 'node:path'
import { brotliDecompressSync, gunzipSync, inflateSync } from 'node:zlib'

const upstream = process.env.UPSTREAM ?? 'http://logdock:8080'
const out = process.env.OUT ?? '/captured'
mkdirSync(out, { recursive: true })
let count = 0

function decompress(body, encoding) {
  try {
    if (encoding === 'gzip') return gunzipSync(body)
    if (encoding === 'deflate') return inflateSync(body)
    if (encoding === 'br') return brotliDecompressSync(body)
  } catch {
    // Kept as it came.
  }
  return body
}

/** The item types of an envelope, for the file's name. */
function kinds(envelope) {
  const types = new Set()
  for (const line of envelope.toString('utf8').split('\n')) {
    try {
      const header = JSON.parse(line)
      if (header && typeof header.type === 'string') types.add(header.type)
    } catch {
      // A payload line that is no JSON header.
    }
  }
  return [...types].join('+') || 'request'
}

createServer(async (request, response) => {
  const chunks = []
  for await (const chunk of request) chunks.push(chunk)
  const body = Buffer.concat(chunks)
  const plain = decompress(body, request.headers['content-encoding'])
  count += 1
  const name = `${String(Date.now())}-${String(count).padStart(4, '0')}-${kinds(plain)}`
  writeFileSync(join(out, `${name}.envelope`), plain)
  writeFileSync(
    join(out, `${name}.json`),
    JSON.stringify({ method: request.method, path: request.url, headers: request.headers }, null, 2),
  )
  const headers = { ...request.headers }
  delete headers.host
  delete headers['content-length']
  try {
    const answer = await fetch(new URL(request.url, upstream), {
      method: request.method,
      headers,
      body: ['GET', 'HEAD'].includes(request.method) ? undefined : body,
    })
    response.writeHead(answer.status, Object.fromEntries(answer.headers))
    response.end(Buffer.from(await answer.arrayBuffer()))
  } catch (error) {
    response.writeHead(502, { 'content-type': 'text/plain' })
    response.end(String(error))
  }
}).listen(8081, () => console.log(`capturing to ${out}, passing on to ${upstream}`))
