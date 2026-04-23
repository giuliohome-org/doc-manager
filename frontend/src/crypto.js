// Client-side, zero-knowledge encryption for documents.
// AES-256-GCM with PBKDF2-SHA256 (600k iterations). Password never leaves the browser.

const TEXT_PREFIX = 'DMENC1:';
const FILE_BLOB_NAME = 'dmencblob';
const FILE_MAGIC = new Uint8Array([0x44, 0x4d, 0x45, 0x4e, 0x43, 0x31, 0x00]); // "DMENC1\0"
const ITERATIONS = 600_000;
const SALT_LEN = 16;
const IV_LEN = 12;
const KEY_BITS = 256;

async function deriveKey(password, salt) {
  const material = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(password),
    'PBKDF2',
    false,
    ['deriveKey']
  );
  return crypto.subtle.deriveKey(
    { name: 'PBKDF2', salt, iterations: ITERATIONS, hash: 'SHA-256' },
    material,
    { name: 'AES-GCM', length: KEY_BITS },
    false,
    ['encrypt', 'decrypt']
  );
}

function bytesToBase64(bytes) {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

function base64ToBytes(b64) {
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

export async function encryptText(password, plaintext) {
  const salt = crypto.getRandomValues(new Uint8Array(SALT_LEN));
  const iv = crypto.getRandomValues(new Uint8Array(IV_LEN));
  const key = await deriveKey(password, salt);
  const ct = new Uint8Array(
    await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, new TextEncoder().encode(plaintext))
  );
  const combined = new Uint8Array(salt.length + iv.length + ct.length);
  combined.set(salt, 0);
  combined.set(iv, salt.length);
  combined.set(ct, salt.length + iv.length);
  return TEXT_PREFIX + bytesToBase64(combined);
}

export async function decryptText(password, envelope) {
  if (!isEncryptedText(envelope)) throw new Error('Not an encrypted payload');
  const combined = base64ToBytes(envelope.slice(TEXT_PREFIX.length));
  if (combined.length < SALT_LEN + IV_LEN + 16) throw new Error('Payload truncated');
  const salt = combined.slice(0, SALT_LEN);
  const iv = combined.slice(SALT_LEN, SALT_LEN + IV_LEN);
  const ct = combined.slice(SALT_LEN + IV_LEN);
  const key = await deriveKey(password, salt);
  const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, ct);
  return new TextDecoder().decode(pt);
}

// Encrypts a File/Blob. Plaintext layout: [2-byte BE name length][name UTF-8][file bytes].
// Wire layout:       [7-byte magic][16 salt][12 iv][ciphertext+tag].
// Returns a File with name FILE_BLOB_NAME (no extension) so Rocket's TempFile.name() preserves it.
export async function encryptFile(password, file) {
  const salt = crypto.getRandomValues(new Uint8Array(SALT_LEN));
  const iv = crypto.getRandomValues(new Uint8Array(IV_LEN));
  const key = await deriveKey(password, salt);
  const fileBytes = new Uint8Array(await file.arrayBuffer());
  const nameBytes = new TextEncoder().encode(file.name);
  if (nameBytes.length > 0xffff) throw new Error('Filename too long to encrypt');
  const plaintext = new Uint8Array(2 + nameBytes.length + fileBytes.length);
  plaintext[0] = (nameBytes.length >> 8) & 0xff;
  plaintext[1] = nameBytes.length & 0xff;
  plaintext.set(nameBytes, 2);
  plaintext.set(fileBytes, 2 + nameBytes.length);
  const ct = new Uint8Array(
    await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, plaintext)
  );
  const wire = new Uint8Array(FILE_MAGIC.length + salt.length + iv.length + ct.length);
  wire.set(FILE_MAGIC, 0);
  wire.set(salt, FILE_MAGIC.length);
  wire.set(iv, FILE_MAGIC.length + salt.length);
  wire.set(ct, FILE_MAGIC.length + salt.length + iv.length);
  return new File([wire], FILE_BLOB_NAME, { type: 'application/octet-stream' });
}

export async function decryptFile(password, encryptedBytes) {
  const bytes = encryptedBytes instanceof Uint8Array ? encryptedBytes : new Uint8Array(encryptedBytes);
  if (bytes.length < FILE_MAGIC.length + SALT_LEN + IV_LEN + 16) {
    throw new Error('Not an encrypted file (truncated)');
  }
  for (let i = 0; i < FILE_MAGIC.length; i++) {
    if (bytes[i] !== FILE_MAGIC[i]) throw new Error('Not an encrypted file (bad magic)');
  }
  const off = FILE_MAGIC.length;
  const salt = bytes.slice(off, off + SALT_LEN);
  const iv = bytes.slice(off + SALT_LEN, off + SALT_LEN + IV_LEN);
  const ct = bytes.slice(off + SALT_LEN + IV_LEN);
  const key = await deriveKey(password, salt);
  const pt = new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, ct));
  const nameLen = (pt[0] << 8) | pt[1];
  const name = new TextDecoder().decode(pt.slice(2, 2 + nameLen));
  const data = pt.slice(2 + nameLen);
  return { name, bytes: data };
}

export function isEncryptedText(content) {
  return typeof content === 'string' && content.startsWith(TEXT_PREFIX);
}

// A document's attachment is encrypted iff its file_id is `{docId}_dmencblob`.
export function isEncryptedFileId(fileId, docId) {
  return typeof fileId === 'string' && fileId === `${docId}_${FILE_BLOB_NAME}`;
}

export const ENC_FILE_BLOB_NAME = FILE_BLOB_NAME;
