import '@testing-library/jest-dom';

const mockLocation = {
  pathname: '/',
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

beforeEach(() => {
  global.fetch = vi.fn();
  delete window.location;
  window.location = { ...mockLocation };
});

afterEach(() => {
  vi.restoreAllMocks();
});