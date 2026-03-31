import { Routes, Route } from 'react-router-dom';
import { RootLayout } from './routes/RootLayout';
import { JobsPage } from './routes/JobsPage';
import { SettingsPage } from './routes/SettingsPage';

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<RootLayout />}>
        <Route index element={<JobsPage />} />
        <Route path="jobs/:jobId" element={<JobsPage />} />
        <Route path="settings" element={<SettingsPage />} />
      </Route>
    </Routes>
  );
}
