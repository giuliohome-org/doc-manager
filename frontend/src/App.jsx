import { useState, useEffect } from 'react';
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from '@tanstack/react-query'
import { BrowserRouter as Router, Routes, Route, Link } from 'react-router-dom';
import './App.css'
import {
  encryptText, decryptText, encryptFile, decryptFile,
  isEncryptedText, isEncryptedFileId,
} from './crypto';

const queryClient = new QueryClient()

function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <Router>
        <div className="min-h-screen bg-gray-100">
          <nav className="bg-white shadow-lg">
            <div className="max-w-7xl mx-auto px-4">
              <div className="flex justify-between items-center py-4">
                <Link style={{ colorScheme: 'light' }}
                  to="/" className="text-2xl font-bold text-blue-600">
                  DocManager
                </Link>
                <Link
                  to="/new"
                  className="bg-blue-500 hover:bg-blue-600 text-white-custom px-4 py-2 rounded-lg transition-colors"
                >
                  New Document
                </Link>
              </div>
            </div>
          </nav>

          <Routes>
            <Route path="/" element={<DocumentList />} />
            <Route path="/new" element={<DocumentEditor />} />
            <Route path="/edit/:id" element={<DocumentEditor />} />
            <Route path="/view/:id" element={<DocumentViewer />} />
          </Routes>
        </div>
      </Router>
    </QueryClientProvider>
  );
}

const backendUrl = import.meta.env.VITE_BACKEND_URL || "/api";
const MAX_UPLOAD_BYTES = 10 * 1024 * 1024;

function triggerDownload(bytes, filename) {
  const blob = new Blob([bytes], { type: 'application/octet-stream' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

// eslint-disable-next-line react/prop-types
function UnlockPanel({ title, onUnlock, errorMessage, busy }) {
  const [pw, setPw] = useState('');
  return (
    <div className="max-w-md mx-auto px-4 py-8">
      <form
        onSubmit={(e) => { e.preventDefault(); if (pw) onUnlock(pw); }}
        style={{ colorScheme: 'light' }}
        className="bg-white rounded-lg shadow-md p-6 text-black"
      >
        <h2 className="text-2xl font-bold mb-2">🔒 {title}</h2>
        <p className="text-sm text-gray-600 mb-4">
          Questo documento è cifrato. Inserisci la password per decifrarlo.
          La password non viene mai inviata al server.
        </p>
        <input
          type="password"
          autoFocus
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          placeholder="Password"
          className="w-full px-4 py-2 border rounded-lg mb-3 text-black"
        />
        {errorMessage && <p className="text-red-600 text-sm mb-3">{errorMessage}</p>}
        <div className="flex justify-end space-x-3">
          <Link to="/" className="px-4 py-2 border rounded-lg hover:bg-gray-50">Cancel</Link>
          <button
            type="submit"
            disabled={busy || !pw}
            style={{
              backgroundColor: busy || !pw ? '#cccccc' : '#000000',
              color: '#FFFFFF',
            }}
            className="px-4 py-2 rounded-lg"
          >
            {busy ? 'Unlocking…' : 'Unlock'}
          </button>
        </div>
      </form>
    </div>
  );
}

export function DocumentList() {
  const [confirmDeleteId, setConfirmDeleteId] = useState(null);
  const [searchQuery, setSearchQuery] = useState('');
  const { isPending, error, data, isFetching } = useQuery({
    queryKey: ['docListData'],
    queryFn: async () => {
      const response = await fetch(`${backendUrl}/documents`);
      if (!response.ok) {
        const errorData = await response.json();
        throw new Error(errorData.message || 'An error occurred');
      }
      return await response.json();
    },
  });

  if (isPending) return <div className="text-center py-8">Loading...</div>;
  if (error) return <div className="text-center py-8">{error.message}</div>;
  if (isFetching) return <div className="text-center py-8">Updating...</div>;

  const deleteDocument = (id) => {
    fetch(`${backendUrl}/documents/${id}`, { method: 'DELETE' })
      .then(() => window.location.href = '/');
  };

  const confirmingDoc = confirmDeleteId
    ? data.find(d => d.id === confirmDeleteId)
    : null;

  const filtered = data.filter(doc => {
    const query = searchQuery.toLowerCase();
    return doc.content.toLowerCase().includes(query)
      || doc.title.toLowerCase().includes(query);
  });

  return (
    <div className="max-w-7xl mx-auto px-4 py-8">
      <div className="mb-6">
        <input
          type="text"
          placeholder="Search documents..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="w-full max-w-md px-4 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent text-black"
        />
      </div>
      {filtered.length === 0 && searchQuery && (
        <p className="text-gray-500 text-center py-8">No documents match your search.</p>
      )}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {filtered.map(doc => {
          const encrypted = isEncryptedText(doc.content);
          const fileEncrypted = isEncryptedFileId(doc.file_id, doc.id);
          return (
          <div key={doc.id}
            style={{ colorScheme: 'light' }}
            className="bg-white rounded-lg shadow-md p-6 hover:shadow-lg transition-shadow text-black">
            <div className="flex justify-between items-start mb-4">
              <h3 className="text-lg font-semibold truncate">
                {encrypted && <span title="Encrypted" className="mr-1">🔒</span>}
                {doc.title || `Document ${doc.id.slice(0, 8)}`}
              </h3>
              <div className="flex space-x-2">
                <Link style={{ colorScheme: 'light' }}
                  to={`/edit/${doc.id}`}
                  className="text-blue-500 hover:text-blue-600">
                  Edit
                </Link>
                <button
                  onClick={() => setConfirmDeleteId(doc.id)}
                  className="text-red-500 hover:text-red-600">
                  Delete
                </button>
              </div>
            </div>
            {encrypted ? (
              <p className="text-gray-500 italic mb-4">
                Encrypted document — open to unlock
              </p>
            ) : (
              <>
                <a href={`${backendUrl}/documents/download/${doc.id}`} className="text-blue-500 hover:text-blue-600">
                  Download Text File
                </a>
                <p style={{ colorScheme: 'light' }}
                  className="text-gray-600 line-clamp-3 mb-4">{doc.content}</p>
              </>
            )}

            {doc.file_id && !fileEncrypted && !encrypted && (
              <a href={`${backendUrl}/documents/download/${doc.file_id}`} className="text-blue-500 hover:text-blue-600">
                Download  {doc.file_id}
              </a>
            )}
            {doc.file_id && fileEncrypted && (
              <span className="text-gray-500 text-sm">🔒 Encrypted attachment — open to download</span>
            )}

            <hr className="my-4 border-gray-300" />

            <Link style={{ colorScheme: 'light' }}
              to={`/view/${doc.id}`}
              className="text-gray-500 hover:text-gray-600 text-sm">
              View full document
            </Link>

          </div>
          );
        })}
      </div>

      {confirmingDoc && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          onClick={() => setConfirmDeleteId(null)}
        >
          <div
            style={{ colorScheme: 'light' }}
            className="bg-white text-black rounded-xl shadow-2xl p-6 max-w-sm w-full mx-4 transform transition-all"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-start space-x-4">
              <div className="flex-shrink-0 w-12 h-12 rounded-full bg-red-100 flex items-center justify-center">
                <svg className="w-6 h-6 text-red-600" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
                </svg>
              </div>
              <div className="flex-1">
                <h3 className="text-lg font-semibold">Delete document?</h3>
                <p className="text-sm text-gray-600 mt-1">
                  Document {confirmingDoc.title || confirmingDoc.id.slice(0, 8)} will be permanently deleted. This action cannot be undone.
                </p>
              </div>
            </div>
            <div className="flex justify-end space-x-3 mt-6">
              <button
                onClick={() => setConfirmDeleteId(null)}
                style={{ backgroundColor: '#ffffff', color: '#111827' }}
                className="px-4 py-2 rounded-lg border border-gray-300 hover:bg-gray-50 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={() => {
                  const id = confirmDeleteId;
                  setConfirmDeleteId(null);
                  deleteDocument(id);
                }}
                style={{ backgroundColor: '#dc2626', color: '#ffffff' }}
                className="px-4 py-2 rounded-lg hover:opacity-90 transition-opacity font-semibold"
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export function DocumentEditor() {
  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [file, setFile] = useState(null);
  const [isNew] = useState(!window.location.pathname.includes('edit'));
  const [editing, setEditing] = useState(false);
  const id = window.location.pathname.split('/').pop();

  const [encryptEnabled, setEncryptEnabled] = useState(false);
  const [password, setPassword] = useState('');
  const [passwordConfirm, setPasswordConfirm] = useState('');

  const [lockedContent, setLockedContent] = useState(null);
  const [unlocked, setUnlocked] = useState(false);
  const [unlockError, setUnlockError] = useState(null);
  const [unlockBusy, setUnlockBusy] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const { isPending, error, data, isFetching } = useQuery({
    queryKey: ['singleDocData_' + id],
    queryFn: async () => {
      if (isNew || editing) {
        return { content };
      }
      const response = await fetch(`${backendUrl}/documents/${id}`);
      const retrieved = await response.json();
      return { title: retrieved.title, content: retrieved.content, file_id: retrieved.file_id };
    },
  });

  useEffect(() => {
    if (!data || editing || unlocked) return;
    if (isEncryptedText(data.content) && lockedContent !== data.content) {
      setLockedContent(data.content);
      setTitle(data.title || '');
    } else if (!isEncryptedText(data.content) && content !== data.content) {
      setContent(data.content);
      setTitle(data.title || '');
    }
  }, [data, editing, unlocked, lockedContent, content]);

  const tryUnlock = async (pw) => {
    setUnlockBusy(true);
    setUnlockError(null);
    try {
      const plain = await decryptText(pw, lockedContent);
      setContent(plain);
      setPassword(pw);
      setEncryptEnabled(true);
      setUnlocked(true);
    } catch {
      setUnlockError('Password errata o dati corrotti.');
    } finally {
      setUnlockBusy(false);
    }
  };

  const handleSubmit = async (e) => {
    e.preventDefault();
    if (file && file.size > MAX_UPLOAD_BYTES) {
      const sizeMB = (file.size / 1024 / 1024).toFixed(1);
      const limitMB = MAX_UPLOAD_BYTES / 1024 / 1024;
      window.alert(`File is too large (${sizeMB} MB). Maximum allowed size is ${limitMB} MB.`);
      return;
    }
    if (encryptEnabled) {
      if (!password) {
        window.alert('Password required to encrypt.');
        return;
      }
      if (!unlocked && password !== passwordConfirm) {
        window.alert('Passwords do not match.');
        return;
      }
    }

    setSubmitting(true);
    try {
      let payloadContent = content;
      let payloadFile = file;
      if (encryptEnabled) {
        payloadContent = await encryptText(password, content);
        if (file) payloadFile = await encryptFile(password, file);
      }

      const url = isNew ? `${backendUrl}/documents` : `${backendUrl}/documents/${id}`;
      const method = isNew ? 'POST' : 'PUT';
      setEditing(false);

      const formData = new FormData();
      formData.append('content', payloadContent);
      if (title) formData.append('title', title);
      if (payloadFile) formData.append('file', payloadFile);

      await fetch(url, { method, body: formData });
      window.location.href = '/';
    } catch (err) {
      window.alert('Encryption or upload failed: ' + (err.message || err));
      setSubmitting(false);
    }
  };

  if (!isNew && lockedContent && !unlocked) {
    return <UnlockPanel
      title={data.title || "Unlock document to edit"}
      onUnlock={tryUnlock}
      errorMessage={unlockError}
      busy={unlockBusy}
    />;
  }

  if (isPending) return <div className="text-center py-8">Loading...</div>;
  if (error) return <div className="text-center py-8">{error.message}</div>;
  if (isFetching) return <div className="text-center py-8">Updating...</div>;

  const existingFileId = data && data.file_id;
  const existingFileEncrypted = isEncryptedFileId(existingFileId, id);
  const showEncryptToggle = isNew;

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <form onSubmit={handleSubmit}
        className="bg-white rounded-lg shadow-md p-6">
        <h2 style={{ colorScheme: 'light' }}
          className="text-2xl font-bold mb-6 text-black">
          {isNew ? 'New Document' : 'Edit Document'}
          {title && <span className="ml-2 text-gray-600 font-normal text-lg">— {title}</span>}
          {unlocked && <span className="ml-2 text-sm text-green-700">🔓 unlocked</span>}
        </h2>
        <input style={{ colorScheme: 'light' }}
          type="text"
          value={title}
          onChange={(e) => {
            setEditing(true);
            setTitle(e.target.value);
          }}
          className="w-full p-3 mb-4 border rounded-lg text-black focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          placeholder="Title (optional, always visible)"
        />
        <textarea style={{ colorScheme: 'light', textAlign: 'left' }}
          value={content}
          onChange={(e) => {
            setEditing(true);
            setContent(e.target.value);
          }}
          className="w-full h-64 p-4 border rounded-lg mb-6 resize-none focus:ring-2 focus:ring-blue-500 focus:border-transparent text-black"
          placeholder="Start writing your document here..."
        />
        <input
          type="file"
          onChange={(e) => {
            setEditing(true);
            setFile(e.target.files[0]);
          }}
          className="mb-6"
        />

        {showEncryptToggle && (
          <div style={{ colorScheme: 'light' }} className="mb-6 p-4 border rounded-lg bg-gray-50 text-black">
            <label className="flex items-center space-x-2 cursor-pointer">
              <input
                type="checkbox"
                checked={encryptEnabled}
                onChange={(e) => setEncryptEnabled(e.target.checked)}
              />
              <span className="font-medium">🔒 Encrypt with a password</span>
            </label>
            {encryptEnabled && (
              <div className="mt-3 space-y-2">
                <p className="text-xs text-gray-600">
                  Text and attachment will be encrypted in your browser (AES-256-GCM).
                  The password is never sent to the server. If you lose it, the document
                  is unrecoverable.
                </p>
                <input
                  type="password"
                  placeholder="Password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full px-3 py-2 border rounded text-black"
                />
                <input
                  type="password"
                  placeholder="Confirm password"
                  value={passwordConfirm}
                  onChange={(e) => setPasswordConfirm(e.target.value)}
                  className="w-full px-3 py-2 border rounded text-black"
                />
              </div>
            )}
          </div>
        )}

        {!isNew && unlocked && (
          <div style={{ colorScheme: 'light' }} className="mb-6 p-3 border rounded-lg bg-green-50 text-sm text-gray-700">
            This document is encrypted. Saving will re-encrypt with the same password.
          </div>
        )}

        <div className="flex justify-end space-x-4">
          <Link
            to="/"
            className="px-4 py-2 border rounded-lg hover:bg-gray-50 transition-colors">
            Cancel
          </Link>
          <button
            type="submit"
            disabled={!content.trim() || submitting}
            style={{
              backgroundColor: (content.trim() && !submitting) ? "#000000" : "#cccccc",
              color: "#FFFFFF",
              WebkitAppearance: "none",
              MozAppearance: "none",
              appearance: "none",
            }}
            className="px-4 py-2 rounded-lg hover:bg-blue-600 transition-colors">
            {submitting ? 'Saving…' : (isNew ? 'Create Document' : 'Save Changes')}
          </button>
        </div>
        {!isNew && existingFileId && !existingFileEncrypted && (
          <a href={`${backendUrl}/documents/download/${id}`} className="text-blue-500 hover:text-blue-600 mt-4 block">
            Download File
          </a>
        )}
      </form>
    </div>
  );
}

export function DocumentViewer() {
  const id = window.location.pathname.split('/').pop();

  const { isPending, error, data, isFetching } = useQuery({
    queryKey: ['singleDocData_' + id],
    queryFn: async () => {
      const response = await fetch(`${backendUrl}/documents/${id}`);
      return await response.json();
    },
  });

  const [plaintext, setPlaintext] = useState(null);
  const [sessionPassword, setSessionPassword] = useState(null);
  const [unlockError, setUnlockError] = useState(null);
  const [unlockBusy, setUnlockBusy] = useState(false);
  const [attachmentBusy, setAttachmentBusy] = useState(false);

  if (isPending) return <div className="text-center py-8">Loading...</div>;
  if (error) return <div className="text-center py-8">{error.message}</div>;
  if (isFetching) return <div className="text-center py-8">Updating...</div>;

  const doc = data;
  const encrypted = isEncryptedText(doc.content);
  const fileEncrypted = isEncryptedFileId(doc.file_id, doc.id);

  const tryUnlock = async (pw) => {
    setUnlockBusy(true);
    setUnlockError(null);
    try {
      const plain = await decryptText(pw, doc.content);
      setPlaintext(plain);
      setSessionPassword(pw);
    } catch {
      setUnlockError('Password errata o dati corrotti.');
    } finally {
      setUnlockBusy(false);
    }
  };

  const downloadDecryptedText = () => {
    if (plaintext == null) return;
    triggerDownload(new TextEncoder().encode(plaintext), `${doc.id.slice(0, 8)}.txt`);
  };

  const downloadDecryptedAttachment = async () => {
    if (!sessionPassword) return;
    setAttachmentBusy(true);
    try {
      const resp = await fetch(`${backendUrl}/documents/download/${doc.file_id}`);
      if (!resp.ok) throw new Error('Download failed');
      const buf = new Uint8Array(await resp.arrayBuffer());
      const { name, bytes } = await decryptFile(sessionPassword, buf);
      triggerDownload(bytes, name);
    } catch (e) {
      window.alert('Failed to decrypt attachment: ' + (e.message || e));
    } finally {
      setAttachmentBusy(false);
    }
  };

  if (encrypted && plaintext == null) {
    return <UnlockPanel
      title={doc.title || "Unlock document"}
      onUnlock={tryUnlock}
      errorMessage={unlockError}
      busy={unlockBusy}
    />;
  }

  const displayContent = encrypted ? plaintext : doc.content;

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <div className="bg-white rounded-lg shadow-md p-6">
        <div className="flex justify-between items-center mb-6">
          <h2 style={{ colorScheme: 'light' }}
            className="text-2xl font-bold text-black">
            {encrypted && <span title="Encrypted" className="mr-1">🔒</span>}
            {doc.title || `Document ${doc.id.slice(0, 8)}`}
          </h2>
          <Link style={{ colorScheme: 'light' }}
            to={`/edit/${doc.id}`}
            className="text-blue-500 hover:text-blue-600">
            Edit
          </Link>
        </div>
        {encrypted ? (
          <button
            type="button"
            onClick={downloadDecryptedText}
            className="text-blue-500 hover:text-blue-600 underline bg-transparent border-0 p-0 cursor-pointer"
          >
            Download Decrypted Text File
          </button>
        ) : (
          <a href={`${backendUrl}/documents/download/${doc.id}`} className="text-blue-500 hover:text-blue-600">
            Download Text File
          </a>
        )}

        <pre style={{ colorScheme: 'light', textAlign: 'left' }}
          className="whitespace-pre-wrap font-sans text-black">{displayContent}</pre>

        {doc.file_id && !fileEncrypted && (
          <a href={`${backendUrl}/documents/download/${doc.file_id}`} className="text-blue-500 hover:text-blue-600">
            Download {doc.file_id}
          </a>
        )}
        {doc.file_id && fileEncrypted && (
          <button
            type="button"
            onClick={downloadDecryptedAttachment}
            disabled={attachmentBusy || !sessionPassword}
            className="text-blue-500 hover:text-blue-600 underline bg-transparent border-0 p-0 cursor-pointer disabled:opacity-50"
          >
            {attachmentBusy ? 'Decrypting…' : '🔒 Download & decrypt attachment'}
          </button>
        )}

      </div>
    </div>
  );
}

export default App;
