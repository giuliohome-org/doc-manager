import { describe, it, expect } from 'vitest';
import {
  encryptText, decryptText,
  encryptFile, decryptFile,
  isEncryptedText, isEncryptedFileId,
} from '../crypto';

describe('crypto: text roundtrip', () => {
  it('encrypts then decrypts a plain string', async () => {
    const envelope = await encryptText('correct-horse', 'hello 🌍');
    expect(envelope.startsWith('DMENC1:')).toBe(true);
    const plain = await decryptText('correct-horse', envelope);
    expect(plain).toBe('hello 🌍');
  });

  it('produces different ciphertext for same plaintext (fresh salt/iv)', async () => {
    const a = await encryptText('pw', 'same text');
    const b = await encryptText('pw', 'same text');
    expect(a).not.toBe(b);
  });

  it('fails on wrong password', async () => {
    const envelope = await encryptText('right', 'secret');
    await expect(decryptText('wrong', envelope)).rejects.toThrow();
  });

  it('fails on truncated payload', async () => {
    const envelope = await encryptText('pw', 'secret');
    const broken = envelope.slice(0, envelope.length - 4);
    await expect(decryptText('pw', broken)).rejects.toThrow();
  });

  it('refuses non-encrypted content', async () => {
    await expect(decryptText('pw', 'plain text')).rejects.toThrow();
  });

  it('handles empty string and multibyte unicode', async () => {
    for (const s of ['', '日本語テスト', 'a'.repeat(10000)]) {
      const e = await encryptText('pw', s);
      expect(await decryptText('pw', e)).toBe(s);
    }
  });
});

describe('crypto: file roundtrip', () => {
  it('encrypts then decrypts a file preserving name and bytes', async () => {
    const bytes = new Uint8Array([1, 2, 3, 4, 5, 254, 255]);
    const file = new File([bytes], 'report.pdf', { type: 'application/pdf' });
    const enc = await encryptFile('pw', file);
    expect(enc.name).toBe('dmencblob');
    const raw = new Uint8Array(await enc.arrayBuffer());
    const { name, bytes: out } = await decryptFile('pw', raw);
    expect(name).toBe('report.pdf');
    expect(Array.from(out)).toEqual(Array.from(bytes));
  });

  it('fails on wrong password for file', async () => {
    const file = new File([new Uint8Array([1, 2, 3])], 'a.bin');
    const enc = await encryptFile('right', file);
    const raw = new Uint8Array(await enc.arrayBuffer());
    await expect(decryptFile('wrong', raw)).rejects.toThrow();
  });

  it('rejects bytes without the magic header', async () => {
    const garbage = new Uint8Array(200);
    await expect(decryptFile('pw', garbage)).rejects.toThrow();
  });

  it('preserves a unicode filename', async () => {
    const file = new File([new Uint8Array([42])], 'relazione-anno-2025-€.pdf');
    const enc = await encryptFile('pw', file);
    const raw = new Uint8Array(await enc.arrayBuffer());
    const { name } = await decryptFile('pw', raw);
    expect(name).toBe('relazione-anno-2025-€.pdf');
  });
});

describe('crypto: format detection', () => {
  it('isEncryptedText recognises the DMENC1: prefix', () => {
    expect(isEncryptedText('DMENC1:abc')).toBe(true);
    expect(isEncryptedText('plain text')).toBe(false);
    expect(isEncryptedText(null)).toBe(false);
    expect(isEncryptedText(undefined)).toBe(false);
  });

  it('isEncryptedFileId only matches the sentinel blob name', () => {
    const id = 'abc-123';
    expect(isEncryptedFileId(`${id}_dmencblob`, id)).toBe(true);
    expect(isEncryptedFileId(`${id}_report.pdf`, id)).toBe(false);
    expect(isEncryptedFileId(null, id)).toBe(false);
    // Another document's encrypted attachment must not match this doc's id.
    expect(isEncryptedFileId('other-id_dmencblob', id)).toBe(false);
  });
});
