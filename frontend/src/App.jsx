import { useState } from 'react';
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from '@tanstack/react-query'
import { BrowserRouter as Router, Routes, Route, Link } from 'react-router-dom';
import './App.css'

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

function DocumentList() {
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

  return (
    <div className="max-w-7xl mx-auto px-4 py-8">
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {data.map(doc => (
          <div key={doc.id}
            style={{ colorScheme: 'light' }}
            className="bg-white rounded-lg shadow-md p-6 hover:shadow-lg transition-shadow text-black">
            <div className="flex justify-between items-start mb-4">
              <h3 className="text-lg font-semibold truncate">Document {doc.id.slice(0, 8)}</h3>
              <div className="flex space-x-2">
                <Link style={{ colorScheme: 'light' }}
                  to={`/edit/${doc.id}`}
                  className="text-blue-500 hover:text-blue-600">
                  Edit
                </Link>
                <button
                  onClick={() => deleteDocument(doc.id)}
                  className="text-red-500 hover:text-red-600">
                  Delete
                </button>
              </div>
            </div>
            <a href={`${backendUrl}/documents/download/${doc.id}`} className="text-blue-500 hover:text-blue-600">
              Download Text File
            </a>

            <p style={{ colorScheme: 'light' }}
              className="text-gray-600 line-clamp-3 mb-4">{doc.content}</p>


            {doc.file_id && <a href={`${backendUrl}/documents/download/${doc.file_id}`} className="text-blue-500 hover:text-blue-600">
              Download  {doc.file_id}
            </a>}

            <hr className="my-4 border-gray-300" />

            <Link style={{ colorScheme: 'light' }}
              to={`/view/${doc.id}`}
              className="text-gray-500 hover:text-gray-600 text-sm">
              View full document
            </Link>

          </div>
        ))}
      </div>
    </div>
  );
}

function DocEditRender(content, handleSubmit, isNew, setContent, setEditing, file, setFile, id, isBinary) {
  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <form onSubmit={handleSubmit}
        className="bg-white rounded-lg shadow-md p-6">
        <h2 style={{ colorScheme: 'light' }}
          className="text-2xl font-bold mb-6 text-black">
          {isNew ? 'New Document' : 'Edit Document'}
        </h2>
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
        <div className="flex justify-end space-x-4">
          <Link
            to="/"
            className="px-4 py-2 border rounded-lg hover:bg-gray-50 transition-colors">
            Cancel
          </Link>
          <button
            type="submit"
            disabled={!content.trim()} // Disable button if content is empty
            style={{
              backgroundColor: content.trim() ? "#000000" : "#cccccc", // Change color if disabled
              color: "#FFFFFF", // Explicitly force white text
              WebkitAppearance: "none", // Prevents automatic styling in some browsers
              MozAppearance: "none",
              appearance: "none",
            }}
            className="px-4 py-2 rounded-lg hover:bg-blue-600 transition-colors">
            {isNew ? 'Create Document' : 'Save Changes'}
          </button>
        </div>
        {!isNew && (
          <a href={`${backendUrl}/documents/download/${id}`} className="text-blue-500 hover:text-blue-600 mt-4 block">
            Download {isBinary ? 'Binary File' : 'Text File'}
          </a>
        )}
      </form>
    </div>
  );
}

function DocumentEditor() {
  const [content, setContent] = useState('');
  const [file, setFile] = useState(null);
  const [isNew] = useState(!window.location.pathname.includes('edit'));
  const [editing, setEditing] = useState(false);
  const id = window.location.pathname.split('/').pop();

  const { isPending, error, data, isFetching } = useQuery({
    queryKey: ['singleDocData_' + id],
    queryFn: async () => {
      if (isNew || editing) {
        return { content };
      }
      const response = await fetch(`${backendUrl}/documents/${id}`);
      const retrieved = await response.json();
      return { content: retrieved.content, is_binary: retrieved.is_binary };
    },
  });

  const handleSubmit = (e) => {
    e.preventDefault();
    const url = isNew ? `${backendUrl}/documents` : `${backendUrl}/documents/${id}`;
    const method = isNew ? 'POST' : 'PUT';
    setEditing(false);

    const formData = new FormData();
    formData.append('content', content);
    if (file) {
      formData.append('file', file);
    }

    fetch(url, {
      method,
      body: formData,
    }).then(() => {
      window.location.href = '/';
    });
  };

  if (data && !isFetching && !isPending && !error && content !== data.content && !editing) {
    setContent(data.content);
    return DocEditRender(data.content, handleSubmit, isNew, setContent, setEditing, file, setFile, id, data.file_id);
  }

  if (isPending) return <div className="text-center py-8">Loading...</div>;
  if (error) return <div className="text-center py-8">{error.message}</div>;
  if (isFetching) return <div className="text-center py-8">Updating...</div>;

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <form onSubmit={handleSubmit}
        className="bg-white rounded-lg shadow-md p-6">
        <h2 style={{ colorScheme: 'light' }}
          className="text-2xl font-bold mb-6 text-black">
          {isNew ? 'New Document' : 'Edit Document'}
        </h2>
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
        <div className="flex justify-end space-x-4">
          <Link
            to="/"
            className="px-4 py-2 border rounded-lg hover:bg-gray-50 transition-colors">
            Cancel
          </Link>
          <button
            type="submit"
            disabled={!content.trim()} // Disable button if content is empty
            style={{
              backgroundColor: content.trim() ? "#000000" : "#cccccc", // Change color if disabled
              color: "#FFFFFF", // Explicitly force white text
              WebkitAppearance: "none", // Prevents automatic styling in some browsers
              MozAppearance: "none",
              appearance: "none",
            }}
            className="px-4 py-2 rounded-lg hover:bg-blue-600 transition-colors">
            {isNew ? 'Create Document' : 'Save Changes'}
          </button>
        </div>
        {!isNew && (
          <a href={`${backendUrl}/documents/download/${id}`} className="text-blue-500 hover:text-blue-600 mt-4 block">
            Download {data.is_binary ? 'Binary File' : 'Text File'}
          </a>
        )}
      </form>
    </div>
  );
}

function DocumentViewer() {
  const id = window.location.pathname.split('/').pop();

  const { isPending, error, data, isFetching } = useQuery({
    queryKey: ['singleDocData_' + id],
    queryFn: async () => {
      const response = await fetch(`${backendUrl}/documents/${id}`);
      return await response.json();
    },
  });

  if (isPending) return <div className="text-center py-8">Loading...</div>;
  if (error) return <div className="text-center py-8">{error.message}</div>;
  if (isFetching) return <div className="text-center py-8">Updating...</div>;

  const doc = data;

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <div className="bg-white rounded-lg shadow-md p-6">
        <div className="flex justify-between items-center mb-6">
          <h2 style={{ colorScheme: 'light' }}
            className="text-2xl font-bold text-black">Document {doc.id.slice(0, 8)}</h2>
          <Link style={{ colorScheme: 'light' }}
            to={`/edit/${doc.id}`}
            className="text-blue-500 hover:text-blue-600">
            Edit
          </Link>
        </div>
        <a href={`${backendUrl}/documents/download/${doc.id}`} className="text-blue-500 hover:text-blue-600">
          Download Text File
        </a>
        
        <pre style={{ colorScheme: 'light', textAlign: 'left' }}
          className="whitespace-pre-wrap font-sans text-black">{doc.content}</pre>
        
        {doc.file_id && <a href={`${
          backendUrl}/documents/download/${doc.file_id}`} className="text-blue-500 hover:text-blue-600">
          Download {doc.file_id}
        </a>}

      </div>
    </div>
  );
}

export default App;