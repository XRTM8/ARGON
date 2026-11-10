import React from 'react';
import ReactDOM from 'react-dom/client';
import { initSafeAreaInsets } from './core/safeArea';
import App from './App';
import './index.css';
import './argon.css';

initSafeAreaInsets();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
