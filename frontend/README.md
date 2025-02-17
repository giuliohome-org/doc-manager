## Local Dev Setup

# Backend

Set env keys `AZURE_STORAGE_ACCOUNT` and `AZURE_STORAGE_ACCESS_KEY`
```sh
cargo run
```

# Frontend

```sh
cd frontend
set VITE_BACKEND_URL=http://localhost:8000/api
npm run dev
```

Browse http://localhost:5173/