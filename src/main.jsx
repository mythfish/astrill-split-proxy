import React from 'react';
import { createRoot } from 'react-dom/client';
import { App as AntApp, ConfigProvider } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import 'antd/dist/reset.css';
import './styles.css';
import App from './App.jsx';

createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <ConfigProvider
      locale={zhCN}
      theme={{
        token: {
          colorPrimary: '#16795b',
          colorSuccess: '#16795b',
          colorInfo: '#2c6f93',
          colorWarning: '#b36b1f',
          colorError: '#b84a3d',
          colorText: '#18211d',
          colorTextSecondary: '#65746b',
          colorBgLayout: '#eef3ee',
          colorBgContainer: '#fffdf7',
          colorBorder: '#d8ded6',
          borderRadius: 6,
          fontFamily:
            'Avenir Next, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
        },
        components: {
          Button: {
            controlHeight: 36,
            borderRadius: 6,
          },
          Card: {
            borderRadiusLG: 8,
            paddingLG: 18,
          },
          Tabs: {
            cardBg: '#f7faf6',
            itemSelectedColor: '#16795b',
          },
          Table: {
            headerBg: '#f3f6f1',
            rowHoverBg: '#f4f8f4',
          },
        },
      }}
    >
      <AntApp>
        <App />
      </AntApp>
    </ConfigProvider>
  </React.StrictMode>,
);
