## Local Dev Setup

# Backend

Set env vars `AZURE_STORAGE_ACCOUNT` and `AZURE_STORAGE_ACCESS_KEY`
```sh
cargo run
```

# Frontend

Set env var `VITE_BACKEND_URL` as `http://localhost:8000/api`
```sh
cd frontend
npm run dev
```

Browse http://localhost:5173/