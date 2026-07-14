import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  App as AntApp,
  Badge,
  Button,
  Card,
  Col,
  Divider,
  Flex,
  Form,
  Input,
  InputNumber,
  List,
  Row,
  Segmented,
  Space,
  Statistic,
  Switch,
  Table,
  Tabs,
  Tag,
  Typography,
} from 'antd';
import {
  Activity,
  CircleStop,
  ClipboardCheck,
  Compass,
  Eraser,
  Globe2,
  KeyRound,
  Play,
  Radar,
  RefreshCcw,
  Save,
  Search,
  Server,
  ShieldCheck,
  Terminal,
  Trash2,
} from 'lucide-react';

const { Text, Title } = Typography;

const initialConfig = {
  httpPort: 18080,
  socksPort: 18081,
  upstreamPort: 32768,
  defaultRoute: 'direct',
  autoSystemProxy: false,
  shellProxyTarget: 'split_proxy',
  geminiPort: 18082,
  geminiApiKey: '',
  geminiUpstream: 'split_proxy',
  geminiProxyUrl: '',
  proxyRules: '',
  directRules: '',
};

const routeOptions = [
  { label: 'Direct', value: 'direct' },
  { label: 'Proxy', value: 'proxy' },
];

const shellTargetOptions = [
  { label: 'SplitProxy', value: 'split_proxy' },
  { label: 'Astrill', value: 'astrill' },
];

const geminiUpstreamOptions = [
  { label: 'SplitProxy', value: 'split_proxy' },
  { label: 'Astrill', value: 'astrill' },
  { label: 'Custom', value: 'custom' },
  { label: 'Direct', value: 'direct' },
];

function lines(text) {
  return text
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function shortTime(value) {
  if (!value) return '--';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleTimeString();
}

function icon(node) {
  return node;
}

function routeTag(route) {
  if (route === 'proxy') return <Tag color="success">PROXY</Tag>;
  if (route === 'direct') return <Tag color="processing">DIRECT</Tag>;
  return <Tag>--</Tag>;
}

export default function App() {
  const { message } = AntApp.useApp();
  const [activeTab, setActiveTab] = useState('control');
  const [config, setConfig] = useState(initialConfig);
  const [status, setStatus] = useState({
    running: false,
    gemini_running: false,
    system_proxy: '检测中',
    shell_proxy: '检测中',
  });
  const [countries, setCountries] = useState({ direct: '--', proxy: '--' });
  const [logs, setLogs] = useState([]);
  const [traffic, setTraffic] = useState({
    total: 0,
    proxy: 0,
    direct: 0,
    recent: [],
    proxy_hosts: [],
    updated_at: '',
  });
  const [loginItemEnabled, setLoginItemEnabled] = useState(false);
  const [appEntries, setAppEntries] = useState([]);
  const [geminiTest, setGeminiTest] = useState('');
  const [busy, setBusy] = useState('');

  useEffect(() => {
    window.scrollTo({ top: 0, left: 0 });
  }, []);

  const appendLog = useCallback((entry) => {
    const time = new Date().toLocaleTimeString();
    setLogs((old) => [...old.slice(-179), { time, entry }]);
  }, []);

  const refreshStatus = useCallback(async () => {
    const next = await invoke('get_status');
    setStatus(next);
  }, []);

  const refreshTraffic = useCallback(async () => {
    const next = await invoke('get_traffic_stats');
    setTraffic(next);
  }, []);

  const refreshLoginItem = useCallback(async () => {
    const enabled = await invoke('get_login_item_enabled');
    setLoginItemEnabled(Boolean(enabled));
  }, []);

  const refreshAppEntries = useCallback(async () => {
    const entries = await invoke('list_app_proxy_entries');
    setAppEntries(entries ?? []);
  }, []);

  const loadConfig = useCallback(async () => {
    const raw = await invoke('load_config');
    setConfig({
      httpPort: raw.listen?.http_port ?? 18080,
      socksPort: raw.listen?.socks_port ?? 18081,
      upstreamPort: raw.upstream?.port ?? 32768,
      defaultRoute: raw.default_route ?? 'direct',
      autoSystemProxy: Boolean(raw.auto_system_proxy),
      shellProxyTarget: raw.shell_proxy_target === 'astrill' ? 'astrill' : 'split_proxy',
      geminiPort: raw.gemini?.port ?? 18082,
      geminiApiKey: raw.gemini?.api_key ?? '',
      geminiUpstream: ['split_proxy', 'astrill', 'custom', 'direct'].includes(raw.gemini?.upstream) ? raw.gemini.upstream : 'split_proxy',
      geminiProxyUrl: raw.gemini?.proxy_url ?? '',
      proxyRules: (raw.rules?.proxy ?? []).join('\n'),
      directRules: (raw.rules?.direct ?? []).join('\n'),
    });
    await refreshStatus();
  }, [refreshStatus]);

  const runAction = useCallback(
    async (key, label, task) => {
      if (busy) return;
      setBusy(key);
      appendLog(label);
      try {
        await task();
        await refreshStatus();
      } catch (error) {
        message.error(String(error));
        appendLog(`ERROR: ${error}`);
      } finally {
        setBusy('');
      }
    },
    [appendLog, busy, message, refreshStatus],
  );

  const saveConfig = useCallback(async () => {
    await invoke('save_config', {
      req: {
        http_port: Number(config.httpPort),
        socks_port: Number(config.socksPort),
        upstream_port: Number(config.upstreamPort),
        default_route: config.defaultRoute,
        auto_system_proxy: Boolean(config.autoSystemProxy),
        shell_proxy_target: config.shellProxyTarget,
        gemini_port: Number(config.geminiPort),
        gemini_api_key: config.geminiApiKey,
        gemini_upstream: config.geminiUpstream,
        gemini_proxy_url: config.geminiProxyUrl,
        proxy_rules: lines(config.proxyRules),
        direct_rules: lines(config.directRules),
      },
    });
    appendLog('配置已保存');
  }, [appendLog, config]);

  useEffect(() => {
    loadConfig().catch((error) => {
      appendLog(`ERROR: ${error}`);
      message.error(String(error));
    });
    refreshLoginItem().catch(() => {});
    refreshAppEntries().catch(() => {});
    const timer = window.setInterval(() => {
      refreshStatus().catch(() => {});
    }, 5000);
    return () => window.clearInterval(timer);
  }, [appendLog, loadConfig, message, refreshAppEntries, refreshLoginItem, refreshStatus]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (activeTab === 'monitor') refreshTraffic().catch(() => {});
    }, 2000);
    return () => window.clearInterval(timer);
  }, [activeTab, refreshTraffic]);

  useEffect(() => {
    let dispose;
    listen('log', (event) => appendLog(String(event.payload))).then((fn) => {
      dispose = fn;
    });
    return () => {
      if (dispose) dispose();
    };
  }, [appendLog]);

  const trafficColumns = useMemo(
    () => [
      { title: '时间', dataIndex: 'ts', width: 104, render: shortTime },
      { title: '走向', dataIndex: 'route', width: 94, render: routeTag },
      { title: '协议', dataIndex: 'protocol', width: 92 },
      {
        title: 'Host',
        dataIndex: 'host',
        ellipsis: true,
        render: (value) => <Text strong>{value || '--'}</Text>,
      },
      { title: '端口', dataIndex: 'port', width: 78 },
      { title: '方法', dataIndex: 'method', width: 94 },
    ],
    [],
  );

  const isBusy = Boolean(busy);
  const proxyRunning = Boolean(status.running);
  const geminiRunning = Boolean(status.gemini_running);
  const configLocked = isBusy || proxyRunning;
  const geminiLocked = isBusy || geminiRunning;
  const systemProxyOn = status.system_proxy === '已开启';
  const systemProxyOff = status.system_proxy === '已关闭';
  const shellProxyOn = status.shell_proxy === '已配置';
  const shellProxyOff = status.shell_proxy === '未配置';
  const hasTraffic = Number(traffic.total ?? 0) > 0;
  const shellTargetLabel = config.shellProxyTarget === 'astrill' ? `Astrill:${config.upstreamPort}` : `SplitProxy:${config.httpPort}/${config.socksPort}`;
  const geminiBaseUrl = `http://127.0.0.1:${config.geminiPort}/v1`;

  const controlTab = (
    <div className="tab-body">
      <Row gutter={[16, 16]} align="stretch">
        <Col xs={24} xl={14}>
          <Card
            className="section-card"
            title={
              <Space>
                {icon(<Radar size={18} />)}
              运行配置
              </Space>
            }
            extra={<Text type="secondary">系统代理：{status.system_proxy}</Text>}
          >
            <Form layout="vertical">
              <Row gutter={12}>
                <Col span={8}>
                  <Form.Item label="HTTP">
                    <InputNumber
                      min={1}
                      max={65535}
                      value={config.httpPort}
                      disabled={configLocked}
                      onChange={(value) => setConfig((old) => ({ ...old, httpPort: value ?? 18080 }))}
                      className="full"
                    />
                  </Form.Item>
                </Col>
                <Col span={8}>
                  <Form.Item label="SOCKS5">
                    <InputNumber
                      min={1}
                      max={65535}
                      value={config.socksPort}
                      disabled={configLocked}
                      onChange={(value) => setConfig((old) => ({ ...old, socksPort: value ?? 18081 }))}
                      className="full"
                    />
                  </Form.Item>
                </Col>
                <Col span={8}>
                  <Form.Item label="Astrill">
                    <InputNumber
                      min={1}
                      max={65535}
                      value={config.upstreamPort}
                      disabled={configLocked}
                      onChange={(value) => setConfig((old) => ({ ...old, upstreamPort: value ?? 32768 }))}
                      className="full"
                    />
                  </Form.Item>
                </Col>
              </Row>
              <Flex justify="space-between" align="center" gap={12} className="route-line">
                <Text type="secondary">默认走向</Text>
                <Segmented
                  options={routeOptions}
                  value={config.defaultRoute}
                  disabled={configLocked}
                  onChange={(value) => setConfig((old) => ({ ...old, defaultRoute: value }))}
                />
              </Flex>
              <Flex justify="space-between" align="center" gap={12} className="route-line">
                <Text type="secondary">启动代理后自动开启系统代理</Text>
                <Switch
                  checked={config.autoSystemProxy}
                  disabled={configLocked}
                  onChange={(checked) => setConfig((old) => ({ ...old, autoSystemProxy: checked }))}
                />
              </Flex>
              <Divider />
              <Flex wrap gap={8}>
                <Button
                  icon={icon(<Search size={16} />)}
                  loading={busy === 'detect'}
                  disabled={isBusy || proxyRunning}
                  onClick={() =>
                    runAction('detect', '检测 Astrill 端口', async () => {
                      const port = await invoke('detect_astrill_port');
                      if (port) setConfig((old) => ({ ...old, upstreamPort: port }));
                    })
                  }
                >
                  检测
                </Button>
                <Button icon={icon(<Save size={16} />)} loading={busy === 'save'} disabled={configLocked} onClick={() => runAction('save', '保存配置', saveConfig)}>
                  保存
                </Button>
                <Button
                  type="primary"
                  icon={icon(<Play size={16} />)}
                  loading={busy === 'start'}
                  disabled={isBusy || proxyRunning}
                  onClick={() =>
                    runAction('start', '启动代理', async () => {
                      await saveConfig();
                      await invoke('start_proxy');
                    })
                  }
                >
                  启动
                </Button>
                <Button danger icon={icon(<CircleStop size={16} />)} loading={busy === 'stop'} disabled={isBusy || !proxyRunning} onClick={() => runAction('stop', '停止代理', () => invoke('stop_proxy'))}>
                  停止
                </Button>
              </Flex>
            </Form>
          </Card>
        </Col>

        <Col xs={24} xl={10}>
          <Card
            className="section-card"
            title={
              <Space>
                {icon(<Globe2 size={18} />)}
              出口验证
              </Space>
            }
            extra={<Text type="secondary">Shell：{status.shell_proxy}</Text>}
          >
            <Row gutter={12}>
              <Col span={12}>
                <Statistic title="Direct" value={countries.direct || '--'} valueStyle={{ color: '#2c6f93' }} />
              </Col>
              <Col span={12}>
                <Statistic title="Proxy" value={countries.proxy || '--'} valueStyle={{ color: '#16795b' }} />
              </Col>
            </Row>
            <Divider />
            <Flex justify="space-between" align="center" gap={12} className="route-line">
              <Text type="secondary">Shell 目标</Text>
              <Segmented
                options={shellTargetOptions}
                value={config.shellProxyTarget}
                disabled={isBusy}
                onChange={(value) => setConfig((old) => ({ ...old, shellProxyTarget: value }))}
              />
            </Flex>
            <Flex wrap gap={8} className="verify-actions">
              <Button
                icon={icon(<ClipboardCheck size={16} />)}
                loading={busy === 'test'}
                disabled={isBusy || !proxyRunning}
                onClick={() =>
                  runAction('test', '出口检测', async () => {
                    const result = await invoke('test_country');
                    setCountries({ direct: result.direct || '--', proxy: result.proxy || '--' });
                  })
                }
              >
                出口
              </Button>
              <Button icon={icon(<ShieldCheck size={16} />)} loading={busy === 'system-on'} disabled={isBusy || !proxyRunning || systemProxyOn} onClick={() => runAction('system-on', '开启系统代理', () => invoke('set_system_proxy', { enabled: true }))}>
                系统开
              </Button>
              <Button danger loading={busy === 'system-off'} disabled={isBusy || systemProxyOff} onClick={() => runAction('system-off', '关闭系统代理', () => invoke('set_system_proxy', { enabled: false }))}>
                系统关
              </Button>
              <Button
                icon={icon(<Terminal size={16} />)}
                loading={busy === 'shell-on'}
                disabled={isBusy}
                onClick={() =>
                  runAction('shell-on', `配置 Shell 代理：${shellTargetLabel}`, async () => {
                    await saveConfig();
                    await invoke('set_shell_proxy', { enabled: true, target: config.shellProxyTarget });
                  })
                }
              >
                Shell 开
              </Button>
              <Button danger loading={busy === 'shell-off'} disabled={isBusy || shellProxyOff} onClick={() => runAction('shell-off', '移除 Shell 代理', () => invoke('set_shell_proxy', { enabled: false }))}>
                Shell 关
              </Button>
            </Flex>
          </Card>
        </Col>

        <Col xs={24} xl={12}>
          <Card className="section-card" title="代理规则" extra={<Text type="secondary">{lines(config.proxyRules).length} 条</Text>}>
            <Input.TextArea
              value={config.proxyRules}
              onChange={(event) => setConfig((old) => ({ ...old, proxyRules: event.target.value }))}
              autoSize={{ minRows: 11, maxRows: 16 }}
              disabled={configLocked}
              spellCheck={false}
              className="rules-box"
            />
          </Card>
        </Col>
        <Col xs={24} xl={12}>
          <Card className="section-card" title="直连规则" extra={<Text type="secondary">{lines(config.directRules).length} 条</Text>}>
            <Input.TextArea
              value={config.directRules}
              onChange={(event) => setConfig((old) => ({ ...old, directRules: event.target.value }))}
              autoSize={{ minRows: 11, maxRows: 16 }}
              disabled={configLocked}
              spellCheck={false}
              className="rules-box"
            />
          </Card>
        </Col>
      </Row>
    </div>
  );

  const monitorTab = (
    <div className="tab-body">
      <Row gutter={[16, 16]}>
        <Col xs={24} md={8}>
          <Card className="stat-card">
            <Statistic title="总量" value={traffic.total ?? 0} prefix={<Activity size={18} />} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card className="stat-card">
            <Statistic title="代理" value={traffic.proxy ?? 0} prefix={<Compass size={18} />} valueStyle={{ color: '#16795b' }} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card className="stat-card">
            <Statistic title="直连" value={traffic.direct ?? 0} prefix={<Globe2 size={18} />} valueStyle={{ color: '#2c6f93' }} />
          </Card>
        </Col>
        <Col xs={24} lg={8}>
          <Card
            className="section-card"
            title="代理域名"
            extra={
              <Space>
                <Button size="small" icon={icon(<RefreshCcw size={14} />)} disabled={isBusy} onClick={refreshTraffic}>
                  刷新
                </Button>
                <Button
                  danger
                  size="small"
                  icon={icon(<Eraser size={14} />)}
                  disabled={isBusy || !hasTraffic}
                  onClick={() =>
                    runAction('clear-traffic', '清空流量记录', async () => {
                      await invoke('clear_traffic_log');
                      await refreshTraffic();
                    })
                  }
                >
                  清空
                </Button>
              </Space>
            }
          >
            <List
              dataSource={traffic.proxy_hosts ?? []}
              locale={{ emptyText: '暂无代理流量' }}
              renderItem={(item) => (
                <List.Item className="host-item">
                  <Text ellipsis>{item.host}</Text>
                  <Badge count={item.count} color="#16795b" />
                </List.Item>
              )}
            />
          </Card>
        </Col>
        <Col xs={24} lg={16}>
          <Card className="section-card" title="最近流量" extra={<Text type="secondary">{traffic.updated_at || '--'}</Text>}>
            <Table
              rowKey={(row, index) => `${row.ts}-${row.host}-${row.port}-${index}`}
              size="small"
              columns={trafficColumns}
              dataSource={traffic.recent ?? []}
              pagination={{ pageSize: 12, size: 'small', showSizeChanger: false }}
              scroll={{ x: 720 }}
            />
          </Card>
        </Col>
      </Row>
    </div>
  );

  const appsTab = (
    <div className="tab-body">
      <Row gutter={[16, 16]}>
        <Col xs={24} lg={10}>
          <Card className="section-card" title="启动设置">
            <Flex justify="space-between" align="center" gap={16} className="settings-line">
              <div>
                <Text strong>开机自动运行</Text>
                <Text type="secondary">后台启动并保留菜单栏图标</Text>
              </div>
              <Switch
                checked={loginItemEnabled}
                loading={busy === 'login-item'}
                disabled={isBusy}
                onChange={(checked) =>
                  runAction('login-item', checked ? '开启开机自启' : '关闭开机自启', async () => {
                    const enabled = await invoke('set_login_item_enabled', { enabled: checked });
                    setLoginItemEnabled(Boolean(enabled));
                  })
                }
              />
            </Flex>
          </Card>
        </Col>
        <Col xs={24} lg={14}>
          <Card
            className="section-card"
            title="应用代理"
            extra={
              <Button
                icon={icon(<Search size={16} />)}
                loading={busy === 'choose-app'}
                disabled={isBusy}
                onClick={() =>
                  runAction('choose-app', '选择代理应用', async () => {
                    const entries = await invoke('choose_app_for_proxy');
                    setAppEntries(entries ?? []);
                  })
                }
              >
                添加应用
              </Button>
            }
          >
            <List
              dataSource={appEntries}
              locale={{ emptyText: '暂无应用' }}
              renderItem={(item) => (
                <List.Item
                  className="app-list-item"
                  actions={[
                    <Button
                      key="launch"
                      type="primary"
                      icon={icon(<Play size={15} />)}
                      loading={busy === `launch-app:${item.id}`}
                      disabled={isBusy || !proxyRunning}
                      onClick={() =>
                        runAction(`launch-app:${item.id}`, `代理启动 ${item.name}`, () => invoke('launch_app_with_proxy', { id: item.id }))
                      }
                    >
                      启动
                    </Button>,
                    <Button
                      key="remove"
                      danger
                      icon={icon(<Trash2 size={15} />)}
                      loading={busy === `remove-app:${item.id}`}
                      disabled={isBusy}
                      onClick={() =>
                        runAction(`remove-app:${item.id}`, `移除 ${item.name}`, async () => {
                          const entries = await invoke('remove_app_proxy_entry', { id: item.id });
                          setAppEntries(entries ?? []);
                        })
                      }
                    >
                      移除
                    </Button>,
                  ]}
                >
                  <List.Item.Meta
                    title={<Text strong>{item.name}</Text>}
                    description={<Text className="app-path">{item.path}</Text>}
                  />
                </List.Item>
              )}
            />
          </Card>
        </Col>
      </Row>
    </div>
  );

  const geminiTab = (
    <div className="tab-body">
      <Row gutter={[16, 16]}>
        <Col xs={24} lg={14}>
          <Card
            className="section-card"
            title={
              <Space>
                {icon(<Server size={18} />)}
                Gemini API
              </Space>
            }
            extra={<Tag color={geminiRunning ? 'success' : 'default'}>{geminiRunning ? 'Running' : 'Stopped'}</Tag>}
          >
            <Form layout="vertical">
              <Row gutter={12}>
                <Col xs={24} md={10}>
                  <Form.Item label="本地端口">
                    <InputNumber
                      min={1}
                      max={65535}
                      value={config.geminiPort}
                      disabled={geminiLocked}
                      onChange={(value) => setConfig((old) => ({ ...old, geminiPort: value ?? 18082 }))}
                      className="full"
                    />
                  </Form.Item>
                </Col>
                <Col xs={24} md={14}>
                  <Form.Item label="出口路径">
                    <Segmented
                      block
                      options={geminiUpstreamOptions}
                      value={config.geminiUpstream}
                      disabled={geminiLocked}
                      onChange={(value) => setConfig((old) => ({ ...old, geminiUpstream: value }))}
                    />
                  </Form.Item>
                </Col>
              </Row>
              {config.geminiUpstream === 'custom' ? (
                <Form.Item label="代理地址">
                  <Input
                    value={config.geminiProxyUrl}
                    disabled={geminiLocked}
                    placeholder="http://127.0.0.1:7890"
                    spellCheck={false}
                    onChange={(event) => setConfig((old) => ({ ...old, geminiProxyUrl: event.target.value }))}
                  />
                </Form.Item>
              ) : null}
              <Form.Item label="Gemini API Key">
                <Input.Password
                  value={config.geminiApiKey}
                  disabled={geminiLocked}
                  prefix={icon(<KeyRound size={15} />)}
                  placeholder="AIza..."
                  autoComplete="off"
                  onChange={(event) => setConfig((old) => ({ ...old, geminiApiKey: event.target.value }))}
                />
              </Form.Item>
              <div className="endpoint-box">
                <Text type="secondary">Base URL</Text>
                <Text code>{geminiBaseUrl}</Text>
              </div>
              <Divider />
              <Flex wrap gap={8}>
                <Button icon={icon(<Save size={16} />)} loading={busy === 'save-gemini'} disabled={geminiLocked} onClick={() => runAction('save-gemini', '保存 Gemini API 配置', saveConfig)}>
                  保存
                </Button>
                <Button
                  type="primary"
                  icon={icon(<Play size={16} />)}
                  loading={busy === 'gemini-start'}
                  disabled={isBusy || geminiRunning || !config.geminiApiKey.trim() || (config.geminiUpstream === 'custom' && !config.geminiProxyUrl.trim())}
                  onClick={() =>
                    runAction('gemini-start', '启动 Gemini API 网关', async () => {
                      await saveConfig();
                      await invoke('start_gemini_api');
                    })
                  }
                >
                  启动
                </Button>
                <Button danger icon={icon(<CircleStop size={16} />)} loading={busy === 'gemini-stop'} disabled={isBusy || !geminiRunning} onClick={() => runAction('gemini-stop', '停止 Gemini API 网关', () => invoke('stop_gemini_api'))}>
                  停止
                </Button>
                <Button
                  icon={icon(<ClipboardCheck size={16} />)}
                  loading={busy === 'gemini-test'}
                  disabled={isBusy || !geminiRunning}
                  onClick={() =>
                    runAction('gemini-test', '测试 Gemini API 网关', async () => {
                      const result = await invoke('test_gemini_api');
                      setGeminiTest(result.output || '');
                      if (!result.ok) message.warning('Gemini 返回了错误，请查看测试结果');
                    })
                  }
                >
                  测试
                </Button>
              </Flex>
            </Form>
          </Card>
        </Col>
        <Col xs={24} lg={10}>
          <Card className="section-card" title="客户端接入">
            <div className="client-snippet">
              <Text type="secondary">OpenAI SDK 兼容模式</Text>
              <pre>{`base_url = "${geminiBaseUrl}"
api_key = "任意非空字符串"
model = "gemini-3.5-flash"`}</pre>
            </div>
            <Divider />
            <Text type="secondary">测试结果</Text>
            <pre className="test-output">{geminiTest || '暂无'}</pre>
          </Card>
        </Col>
      </Row>
    </div>
  );

  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <Text className="eyebrow">OpenWeb split proxy</Text>
          <Title level={2}>Astrill Split Proxy</Title>
        </div>
        <Space size={12}>
          <Tag color={status.running ? 'success' : 'default'} className="run-tag">
            {status.running ? 'Running' : 'Stopped'}
          </Tag>
        </Space>
      </header>

      <Tabs
        activeKey={activeTab}
        onChange={(key) => {
          setActiveTab(key);
          if (key === 'monitor') refreshTraffic().catch(() => {});
        }}
        items={[
          { key: 'control', label: '控制台', children: controlTab },
          { key: 'monitor', label: '流量监控', children: monitorTab },
          { key: 'gemini', label: 'Gemini API', children: geminiTab },
          { key: 'apps', label: '应用代理', children: appsTab },
        ]}
      />

      <Card className="log-card" title="事件日志" extra={<Text type="secondary">{logs.at(-1)?.time ?? 'ready'}</Text>}>
        <div className="log-list">
          {logs.length ? (
            logs.map((item, index) => (
              <div className="log-line" key={`${item.time}-${index}`}>
                <span>{item.time}</span>
                <Text>{item.entry}</Text>
              </div>
            ))
          ) : (
            <Text type="secondary">ready</Text>
          )}
        </div>
      </Card>
    </main>
  );
}
