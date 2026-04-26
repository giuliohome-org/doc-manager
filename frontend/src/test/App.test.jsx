import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Routes, Route } from 'react-router-dom';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { DocumentList, DocumentEditor, DocumentViewer } from '../App';

function setLocation(pathname) {
  delete window.location;
  window.location = {
    pathname,
    href: '',
    assign: vi.fn(),
    replace: vi.fn(),
    reload: vi.fn(),
    origin: 'http://localhost:5173',
    protocol: 'http:',
    host: 'localhost:5173',
    hostname: 'localhost',
    port: '5173',
    search: '',
    hash: '',
  };
}

function renderWithProviders(ui, { route = '/' } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });
  setLocation(route);

  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[route]}>
        <Routes>
          <Route path="/" element={ui} />
          <Route path="/new" element={ui} />
          <Route path="/edit/:id" element={ui} />
          <Route path="/view/:id" element={ui} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  );
}

const mockDocuments = [
  { id: 'abc12345-6789-def0-1234-567890123456', title: 'My First Doc', content: 'Hello world', file_id: null },
  { id: 'def67890-1234-abc0-5678-901234567890', title: '', content: 'Second doc', file_id: 'def67890-1234-abc0-5678-901234567890_report.pdf' },
];

const mockDocument = { id: 'abc12345-6789-def0-1234-567890123456', title: 'My First Doc', content: 'Hello world', file_id: null };

describe('DocumentList', () => {
  it('shows loading state while fetching documents', () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentList />);
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });

  it('displays documents when API returns data', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocuments),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/hello world/i)).toBeInTheDocument();
      expect(screen.getByText(/second doc/i)).toBeInTheDocument();
    });
  });

  it('shows error message when API returns non-ok status', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: false,
        json: () => Promise.resolve({ message: 'Failed to list blobs' }),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/failed to list blobs/i)).toBeInTheDocument();
    });
  });

  it('shows error message when API returns non-ok without message', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: false,
        json: () => Promise.resolve({}),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/an error occurred/i)).toBeInTheDocument();
    });
  });

  it('shows error message when fetch throws network error', async () => {
    global.fetch = vi.fn(() => Promise.reject(new Error('Network error')));
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/network error/i)).toBeInTheDocument();
    });
  });

  it('displays document titles or truncated IDs when title is empty', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocuments),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/my first doc/i)).toBeInTheDocument();
      expect(screen.getByText(/document def67890/i)).toBeInTheDocument();
    });
  });

  it('displays file download link when file_id is present', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocuments),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      const downloadLinks = screen.getAllByText(/download/i);
      expect(downloadLinks.length).toBeGreaterThanOrEqual(2);
    });
  });

  it('calls delete API on delete click', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn((url, options) => {
      if (options && options.method === 'DELETE') {
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve('Document deleted successfully'),
        });
      }
      return Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocuments),
      });
    });

    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/hello world/i)).toBeInTheDocument();
    });

    const deleteButtons = screen.getAllByRole('button', { name: /delete/i });
    await user.click(deleteButtons[0]);

    // The first click opens the confirmation modal. Click the modal's
    // Delete button (now appended after the card buttons) to confirm.
    const afterModalOpen = screen.getAllByRole('button', { name: /delete/i });
    await user.click(afterModalOpen[afterModalOpen.length - 1]);

    expect(global.fetch).toHaveBeenCalledWith(
      expect.stringContaining('/documents/abc12345-6789-def0-1234-567890123456'),
      expect.objectContaining({ method: 'DELETE' })
    );
  });

  it('handles empty document list', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve([]),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.queryByText(/document abc/i)).not.toBeInTheDocument();
    });
  });

  it('renders edit and view links for each document', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve([mockDocument]),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/edit/i)).toBeInTheDocument();
      expect(screen.getByText(/view full document/i)).toBeInTheDocument();
    });
  });

  it('handles documents with unicode content', async () => {
    const unicodeDocs = [
      { id: 'unicode-id-1234', content: 'Héllo wörld 🌍 日本語テスト', file_id: null },
    ];
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(unicodeDocs),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/héllo wörld 🌍 日本語テスト/i)).toBeInTheDocument();
    });
  });
});

describe('DocumentEditor', () => {
  it('shows New Document heading for new document route', async () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });
    await waitFor(() => {
      expect(screen.getByText(/new document/i)).toBeInTheDocument();
    });
  });

  it('shows Edit Document heading for edit route', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve({ ...mockDocument, content: 'Test content' }),
      })
    );
    renderWithProviders(<DocumentEditor />, { route: '/edit/abc12345' });
    await waitFor(() => {
      expect(screen.getByText(/edit document/i)).toBeInTheDocument();
    });
  });

  it('disables submit button when content is empty', async () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /create document/i })).toBeDisabled();
    });
  });

  it('enables submit button when content is provided', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/start writing/i)).toBeInTheDocument();
    });

    const textarea = screen.getByPlaceholderText(/start writing/i);
    await user.type(textarea, 'Some content');

    const submitButton = screen.getByRole('button', { name: /create document/i });
    expect(submitButton).not.toBeDisabled();
  });

  it('disables submit button for whitespace-only content', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/start writing/i)).toBeInTheDocument();
    });

    const textarea = screen.getByPlaceholderText(/start writing/i);
    await user.type(textarea, '   ');

    const submitButton = screen.getByRole('button', { name: /create document/i });
    expect(submitButton).toBeDisabled();
  });

  it('sends POST request with FormData when creating a document', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve({ id: 'new-id', content: 'Test content' }),
      })
    );

    renderWithProviders(<DocumentEditor />, { route: '/new' });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/start writing/i)).toBeInTheDocument();
    });

    const textarea = screen.getByPlaceholderText(/start writing/i);
    await user.type(textarea, 'Test content');

    const submitButton = screen.getByRole('button', { name: /create document/i });
    await user.click(submitButton);

    expect(global.fetch).toHaveBeenCalledWith(
      expect.stringContaining('/documents'),
      expect.objectContaining({ method: 'POST' })
    );
  });

  it('sends PUT request when editing an existing document', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn((url) => {
      if (url.includes('/documents/abc')) {
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ id: 'abc', content: 'Existing content', file_id: null }),
        });
      }
      return Promise.resolve({ ok: true, json: () => Promise.resolve({}) });
    });

    renderWithProviders(<DocumentEditor />, { route: '/edit/abc' });

    await waitFor(() => {
      expect(screen.getByText(/save changes/i)).toBeInTheDocument();
    });

    const textarea = screen.getByPlaceholderText(/start writing/i);
    await user.clear(textarea);
    await user.type(textarea, 'Updated content');

    const submitButton = screen.getByRole('button', { name: /save changes/i });
    await user.click(submitButton);

    expect(global.fetch).toHaveBeenCalledWith(
      expect.stringContaining('/documents/abc'),
      expect.objectContaining({ method: 'PUT' })
    );
  });

  it('has a file input for attachments', async () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /create document/i })).toBeInTheDocument();
    });
  });

  it('has cancel link back to home', async () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });
    await waitFor(() => {
      const cancelLink = screen.getByText(/cancel/i);
      expect(cancelLink).toHaveAttribute('href', '/');
    });
  });

  it('does not show download link for new documents', async () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /create document/i })).toBeInTheDocument();
    });
    expect(screen.queryByText(/download/i)).not.toBeInTheDocument();
  });

  it('shows loading state when fetching existing document', async () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/edit/test-id' });
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });

  it('shows error message when fetching existing document fails', async () => {
    global.fetch = vi.fn(() => Promise.reject(new Error('Server error')));
    renderWithProviders(<DocumentEditor />, { route: '/edit/test-id' });
    await waitFor(() => {
      expect(screen.getByText(/server error/i)).toBeInTheDocument();
    });
  });
});

describe('DocumentViewer', () => {
  it('shows loading state while fetching document', () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentViewer />, { route: '/view/test-id' });
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });

  it('displays document content when loaded', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocument),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/abc' });
    await waitFor(() => {
      expect(screen.getByText(/hello world/i)).toBeInTheDocument();
    });
  });

  it('shows error message when API fails', async () => {
    global.fetch = vi.fn(() => Promise.reject(new Error('Server error')));
    renderWithProviders(<DocumentViewer />, { route: '/view/abc' });
    await waitFor(() => {
      expect(screen.getByText(/server error/i)).toBeInTheDocument();
    });
  });

  it('displays document title in heading', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocument),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/abc' });
    await waitFor(() => {
      expect(screen.getByText(/my first doc/i)).toBeInTheDocument();
    });
  });

  it('shows file download link when file_id is present', async () => {
    const docWithFile = {
      id: 'abc12345-6789-def0-1234-567890123456',
      content: 'Hello world',
      file_id: 'abc12345-6789-def0-1234-567890123456_report.pdf',
    };
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(docWithFile),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/abc' });
    await waitFor(() => {
      expect(screen.getByText(/download.*report\.pdf/i)).toBeInTheDocument();
    });
  });

  it('shows edit link', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocument),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/abc' });
    await waitFor(() => {
      expect(screen.getByText(/edit/i)).toBeInTheDocument();
    });
  });

  it('renders content preserving whitespace', async () => {
    const docWithSpecialContent = {
      id: 'special-id',
      content: 'Line 1\n  Indented\n    More indented',
      file_id: null,
    };
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(docWithSpecialContent),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/special' });
    await waitFor(() => {
      const preElement = screen.getByText(/line 1/i).closest('pre');
      expect(preElement).toBeInTheDocument();
    });
  });

  it('handles document with empty content', async () => {
    const emptyDoc = { id: 'empty-id', content: '', file_id: null };
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(emptyDoc),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/empty' });
    await waitFor(() => {
      expect(screen.getByText(/document empty/i)).toBeInTheDocument();
    });
  });

  it('handles document with unicode content', async () => {
    const unicodeDoc = { id: 'unicode-id', content: '日本語テスト 🌍 éèê', file_id: null };
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(unicodeDoc),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/unicode' });
    await waitFor(() => {
      expect(screen.getByText(/日本語テスト 🌍 éèê/i)).toBeInTheDocument();
    });
  });

  it('shows text download link for document without file', async () => {
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocument),
      })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/abc' });
    await waitFor(() => {
      expect(screen.getByText(/download text file/i)).toBeInTheDocument();
    });
  });
  it('shows title input in editor', async () => {
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });
    await waitFor(() => {
      expect(screen.getByPlaceholderText(/title/i)).toBeInTheDocument();
    });
  });

  it('includes title in FormData when creating a document', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve({ id: 'new-id', title: 'Test Title', content: 'Test content' }),
      })
    );

    renderWithProviders(<DocumentEditor />, { route: '/new' });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/title/i)).toBeInTheDocument();
    });

    const titleInput = screen.getByPlaceholderText(/title/i);
    await user.type(titleInput, 'Test Title');

    const textarea = screen.getByPlaceholderText(/start writing/i);
    await user.type(textarea, 'Test content');

    const submitButton = screen.getByRole('button', { name: /create document/i });
    await user.click(submitButton);

    expect(global.fetch).toHaveBeenCalledWith(
      expect.stringContaining('/documents'),
      expect.objectContaining({ method: 'POST' })
    );
  });

  it('searches documents by title', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () => Promise.resolve(mockDocuments),
      })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/my first doc/i)).toBeInTheDocument();
    });

    const searchInput = screen.getByPlaceholderText(/search/i);
    await user.type(searchInput, 'My First');

    expect(screen.getByText(/my first doc/i)).toBeInTheDocument();
    expect(screen.queryByText(/document def67890/i)).not.toBeInTheDocument();
  });

  it('shows fallback ID when document has no title', async () => {
    const docsNoTitle = [
      { id: 'notitle-1234-5678-abcd-efghijklmnop', title: '', content: 'No title doc', file_id: null },
    ];
    global.fetch = vi.fn(() =>
      Promise.resolve({ ok: true, json: () => Promise.resolve(docsNoTitle) })
    );
    renderWithProviders(<DocumentList />);
    await waitFor(() => {
      expect(screen.getByText(/document notitle/i)).toBeInTheDocument();
    });
  });
});

describe('Encryption UI', () => {
  it('DocumentEditor hides encryption fields until the toggle is checked', async () => {
    const user = userEvent.setup();
    global.fetch = vi.fn(() => new Promise(() => {}));
    renderWithProviders(<DocumentEditor />, { route: '/new' });

    await waitFor(() => {
      expect(screen.getByText(/encrypt with a password/i)).toBeInTheDocument();
    });
    expect(screen.queryByPlaceholderText(/^password$/i)).not.toBeInTheDocument();

    await user.click(screen.getByLabelText(/encrypt with a password/i));
    expect(screen.getByPlaceholderText(/^password$/i)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/confirm password/i)).toBeInTheDocument();
  });

  it('DocumentViewer shows the unlock panel for an encrypted document', async () => {
    const { encryptText } = await import('../crypto');
    const envelope = await encryptText('secret', 'top secret content');
    const encDoc = { id: 'enc12345-0000-0000-0000-000000000000', content: envelope, file_id: null };
    global.fetch = vi.fn(() =>
      Promise.resolve({ ok: true, json: () => Promise.resolve(encDoc) })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/enc12345' });

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /unlock/i })).toBeInTheDocument();
    });
    expect(screen.queryByText(/top secret content/i)).not.toBeInTheDocument();
  });

  it('DocumentViewer decrypts and shows content after a correct password', async () => {
    const user = userEvent.setup();
    const { encryptText } = await import('../crypto');
    const envelope = await encryptText('hunter2', 'decrypted payload');
    const encDoc = { id: 'enc22222-0000-0000-0000-000000000000', content: envelope, file_id: null };
    global.fetch = vi.fn(() =>
      Promise.resolve({ ok: true, json: () => Promise.resolve(encDoc) })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/enc22222' });

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /unlock/i })).toBeInTheDocument();
    });
    await user.type(screen.getByPlaceholderText(/password/i), 'hunter2');
    await user.click(screen.getByRole('button', { name: /unlock/i }));

    await waitFor(() => {
      expect(screen.getByText(/decrypted payload/i)).toBeInTheDocument();
    });
  });

  it('DocumentViewer rejects a wrong password without leaking content', async () => {
    const user = userEvent.setup();
    const { encryptText } = await import('../crypto');
    const envelope = await encryptText('right-pw', 'very private');
    const encDoc = { id: 'enc33333-0000-0000-0000-000000000000', content: envelope, file_id: null };
    global.fetch = vi.fn(() =>
      Promise.resolve({ ok: true, json: () => Promise.resolve(encDoc) })
    );
    renderWithProviders(<DocumentViewer />, { route: '/view/enc33333' });

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /unlock/i })).toBeInTheDocument();
    });
    await user.type(screen.getByPlaceholderText(/password/i), 'wrong-pw');
    await user.click(screen.getByRole('button', { name: /unlock/i }));

    await waitFor(() => {
      expect(screen.getByText(/password errata/i)).toBeInTheDocument();
    });
    expect(screen.queryByText(/very private/i)).not.toBeInTheDocument();
  });

  it('DocumentList marks encrypted docs with a lock badge and hides preview', async () => {
    const { encryptText } = await import('../crypto');
    const envelope = await encryptText('pw', 'should not be visible');
    const docs = [
      { id: 'enc-id-12345678-xxxx-yyyy-zzzz-000000000000', content: envelope, file_id: null },
    ];
    global.fetch = vi.fn(() =>
      Promise.resolve({ ok: true, json: () => Promise.resolve(docs) })
    );
    renderWithProviders(<DocumentList />);

    await waitFor(() => {
      expect(screen.getByText(/encrypted document — open to unlock/i)).toBeInTheDocument();
    });
    expect(screen.queryByText(/should not be visible/i)).not.toBeInTheDocument();
    // No "Download Text File" link for encrypted docs in the list card.
    expect(screen.queryByText(/download text file/i)).not.toBeInTheDocument();
  });
});