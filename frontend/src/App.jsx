import { useState, useEffect } from 'react';
import { BrowserRouter as Router, Routes, Route, Link } from 'react-router-dom';
import './App.css'

function App() {
  return (
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
                className="bg-blue-500 hover:bg-blue-600 text-white px-4 py-2 rounded-lg transition-colors"
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
  );
}

const backendUrl = import.meta.env.VITE_BACKEND_URL || "/api";

function DocumentList() {
  const [documents, setDocuments] = useState([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch(`${backendUrl}/documents`)
      .then(res => res.json())
      .then(data => {
        setDocuments(data);
        setLoading(false);
      });
  }, []);

  const deleteDocument = (id) => {
    fetch(`${backendUrl}/documents/${id}`, { method: 'DELETE' })
      .then(() => setDocuments(docs => docs.filter(d => d.id !== id)));
  };

  if (loading) return <div className="text-center py-8">Loading...</div>;

  return (
    <div className="max-w-7xl mx-auto px-4 py-8">
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {documents.map(doc => (
          <div key={doc.id} 
                style={{ colorScheme: 'light' }}
		className="bg-white rounded-lg shadow-md p-6 hover:shadow-lg transition-shadow text-black">
            <div className="flex justify-between items-start mb-4">
              <h3 className="text-lg font-semibold truncate">Document {doc.id.slice(0, 8)}</h3>
              <div className="flex space-x-2">
                <Link style={{ colorScheme: 'light' }}
                  to={`/edit/${doc.id}`}
                  className="text-blue-500 hover:text-blue-600"
                >
                  Edit
                </Link>
                <button
                  onClick={() => deleteDocument(doc.id)}
                  className="text-red-500 hover:text-red-600"
                >
                  Delete
                </button>
              </div>
            </div>
            <p style={{ colorScheme: 'light' }}
		className="text-gray-600 line-clamp-3 mb-4">{doc.content}</p>
            <Link style={{ colorScheme: 'light' }}
              to={`/view/${doc.id}`}
              className="text-gray-500 hover:text-gray-600 text-sm"
            >
              View full document →
            </Link>
          </div>
        ))}
      </div>
    </div>
  );
}

function DocumentEditor({ match }) {
  const [content, setContent] = useState('');
  const [isNew] = useState(!window.location.pathname.includes('edit'));
  const id = window.location.pathname.split('/').pop();

  useEffect(() => {
    if (!isNew) {
      fetch(`${backendUrl}/documents/${id}`)
        .then(res => res.json())
        .then(data => setContent(data.content));
    }
  }, [id, isNew]);

  const handleSubmit = (e) => {
    e.preventDefault();
    const url = isNew ? `${backendUrl}/documents` : `${backendUrl}/documents/${id}`;
    const method = isNew ? 'POST' : 'PUT';

    fetch(url, {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content }),
    }).then(() => {
      window.location.href = '/';
    });
  };

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <form onSubmit={handleSubmit}
	  className="bg-white rounded-lg shadow-md p-6">
        <h2 style={{ colorScheme: 'light' }} 
	  className="text-2xl font-bold mb-6 text-black">
          {isNew ? 'New Document' : 'Edit Document'}
        </h2>
        <textarea  style={{ colorScheme: 'light' }}
          value={content}
          onChange={(e) => setContent(e.target.value)}
          className="w-full h-64 p-4 border rounded-lg mb-6 resize-none focus:ring-2 focus:ring-blue-500 focus:border-transparent text-black"
          placeholder="Start writing your document here..."
        />
        <div className="flex justify-end space-x-4">
          <Link
            to="/"
            className="px-4 py-2 border rounded-lg hover:bg-gray-50 transition-colors"
          >
            Cancel
          </Link>
          <button
            type="submit"
  style={{
    backgroundColor: "#000000", // Explicitly force black
    color: "#FFFFFF", // Explicitly force white text
    WebkitAppearance: "none", // Prevents automatic styling in some browsers
    MozAppearance: "none",
    appearance: "none",
  }}
            className="px-4 py-2 rounded-lg hover:bg-blue-600 transition-colors"
          >
            {isNew ? 'Create Document' : 'Save Changes'}
          </button>
        </div>
      </form>
    </div>
  );
}

function DocumentViewer() {
  const [doc, setDoc] = useState(null);
  const id = window.location.pathname.split('/').pop();

  useEffect(() => {
    fetch(`${backendUrl}/documents/${id}`)
      .then(res => res.json())
      .then(data => setDoc(data));
  }, [id]);

  if (!doc) return <div className="text-center py-8">Loading...</div>;

  return (
    <div 
	  className="max-w-3xl mx-auto px-4 py-8">
      <div className="bg-white rounded-lg shadow-md p-6">
        <div className="flex justify-between items-center mb-6">
          <h2 style={{ colorScheme: 'light' }} 
	  className="text-2xl font-bold text-black">Document {doc.id.slice(0, 8)}</h2>
          <Link style={{ colorScheme: 'light' }}
            to={`/edit/${doc.id}`}
            className="text-blue-500 hover:text-blue-600"
          >
            Edit
          </Link>
        </div>
        <pre style={{ colorScheme: 'light' }} 
	  className="whitespace-pre-wrap font-sans text-black">{doc.content}</pre>
      </div>
    </div>
  );
}

export default App;
