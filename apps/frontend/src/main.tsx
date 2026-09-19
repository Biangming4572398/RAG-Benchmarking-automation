import React from 'react';
import { createRoot } from 'react-dom/client';
import { BenchmarkDashboard } from './benchmark-dashboard';
import './styles.css';

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BenchmarkDashboard />
  </React.StrictMode>,
);
