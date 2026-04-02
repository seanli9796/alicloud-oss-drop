require('dotenv').config();
const express = require('express');
const multer = require('multer');
const OSS = require('ali-oss');
const {
  DEFAULT_REGION,
  getEffectiveConfig,
  getPublicConfig,
  hasBucketTarget,
  hasOssConfig,
  normalizePrefix,
  saveLocalConfig,
} = require('./config-store');

let cachedClient = null;
let cachedClientSignature = '';
const MAX_PREFIX_TOTAL_ITEMS = 300000;
const MAX_PREFIX_SCAN_PARENTS = 30000;
const MAX_OBJECT_SCAN_PAGES = 5000;
const MAX_OBJECT_SCAN_KEYS = 1000000;

function getRuntimeContext() {
  const config = getEffectiveConfig();

  if (!hasBucketTarget(config)) {
    cachedClient = null;
    cachedClientSignature = '';
    return { config, client: null };
  }

  const signature = [
    config.region,
    config.accessKeyId,
    config.accessKeySecret,
    config.bucket,
  ].join(':');

  if (!cachedClient || cachedClientSignature !== signature) {
    cachedClient = new OSS({
      region: config.region,
      accessKeyId: config.accessKeyId,
      accessKeySecret: config.accessKeySecret,
      bucket: config.bucket,
    });
    cachedClientSignature = signature;
  }

  return { config, client: cachedClient };
}

async function listChildPrefixes(client, parentPrefix) {
  const prefixes = [];
  let continuationToken;

  do {
    const result = await client.listV2({
      prefix: parentPrefix || undefined,
      delimiter: '/',
      'max-keys': 1000,
      'encoding-type': 'url',
      'continuation-token': continuationToken,
    });

    if (Array.isArray(result.prefixes)) {
      prefixes.push(...result.prefixes.map(prefix => {
        try {
          return decodeURIComponent(prefix);
        } catch (_) {
          return prefix;
        }
      }));
    }

    continuationToken = result.nextContinuationToken;
  } while (continuationToken);

  return prefixes;
}

async function listAllPrefixes(client) {
  // 快速通道：按对象分页扫描并提取前缀，通常请求数最少。
  try {
    const scan = await listPrefixesByObjectScan(client, '');
    return {
      prefixes: scan.prefixes.sort((a, b) => a.localeCompare(b, 'zh-Hans-CN')),
      stats: {
        visited: scan.visitedPages,
        fallbackHits: scan.fallbackHits,
        failedParents: [],
        truncated: scan.truncated,
      },
    };
  } catch (fastErr) {
    const discovered = new Set();
    const queued = new Set(['']);
    const queue = [''];
    const failedParents = [`<root> (快速扫描失败: ${fastErr.message || fastErr})`];
    let visitedParents = 0;
    let fallbackHits = 0;
    let truncated = false;

    while (queue.length > 0) {
      if (
        visitedParents >= MAX_PREFIX_SCAN_PARENTS
        || discovered.size >= MAX_PREFIX_TOTAL_ITEMS
      ) {
        truncated = true;
        break;
      }

      visitedParents += 1;
      const parentPrefix = queue.shift() || '';

      try {
        const children = await listChildPrefixes(client, parentPrefix);
        for (const child of children) {
          const normalized = normalizePrefix(child);
          if (!normalized) continue;

          if (!discovered.has(normalized)) {
            discovered.add(normalized);
            if (!queued.has(normalized)) {
              queued.add(normalized);
              queue.push(normalized);
            }
          }

          if (discovered.size >= MAX_PREFIX_TOTAL_ITEMS) {
            truncated = true;
            break;
          }
        }
      } catch (err) {
        try {
          const extraScan = await listPrefixesByObjectScan(client, parentPrefix);
          fallbackHits += 1 + extraScan.fallbackHits;
          for (const prefix of extraScan.prefixes) {
            const normalized = normalizePrefix(prefix);
            if (!normalized) continue;
            discovered.add(normalized);
            if (discovered.size >= MAX_PREFIX_TOTAL_ITEMS) {
              truncated = true;
              break;
            }
          }
        } catch (scanErr) {
          failedParents.push(
            `${parentPrefix || '<root>'} (目录枚举失败: ${err.message || err}; 对象扫描失败: ${scanErr.message || scanErr})`
          );
        }
      }

      if (truncated) break;
    }

    return {
      prefixes: Array.from(discovered).sort((a, b) => a.localeCompare(b, 'zh-Hans-CN')),
      stats: {
        visited: visitedParents,
        fallbackHits,
        failedParents,
        truncated,
      },
    };
  }
}

async function listPrefixesByObjectScan(client, parentPrefix = '') {
  const discovered = new Set();
  let visitedPages = 0;
  let scannedKeys = 0;
  let continuationToken;
  let fallbackHits = 0;
  let truncated = false;

  while (true) {
    if (
      visitedPages >= MAX_OBJECT_SCAN_PAGES
      || scannedKeys >= MAX_OBJECT_SCAN_KEYS
      || discovered.size >= MAX_PREFIX_TOTAL_ITEMS
    ) {
      truncated = true;
      break;
    }

    visitedPages += 1;
    let result;

    try {
      result = await client.listV2({
        prefix: parentPrefix || undefined,
        'max-keys': 1000,
        'encoding-type': 'url',
        'continuation-token': continuationToken,
      });
    } catch (err) {
      // 节点服务里 listV2 只有单通道，无法像 Rust 侧那样做 SDK 解析回退，直接上抛。
      throw err;
    }

    const objects = Array.isArray(result.objects) ? result.objects : [];
    scannedKeys += objects.length;

    for (const obj of objects) {
      const rawKey = String(obj?.name || '');
      if (!rawKey) continue;

      let key = rawKey;
      try {
        key = decodeURIComponent(rawKey);
      } catch (_) {
        key = rawKey;
      }

      if (parentPrefix && !key.startsWith(parentPrefix)) continue;
      collectPrefixesFromKey(key, discovered);
      if (discovered.size >= MAX_PREFIX_TOTAL_ITEMS) {
        break;
      }
    }

    continuationToken = result.nextContinuationToken;
    if (!continuationToken) break;
  }

  return {
    prefixes: Array.from(discovered),
    visitedPages,
    fallbackHits,
    truncated,
  };
}

function collectPrefixesFromKey(key, targetSet) {
  const normalized = String(key || '').trim();
  if (!normalized) return;

  const segments = normalized.split('/').filter(Boolean);
  if (!segments.length) return;

  const includeDepth = normalized.endsWith('/')
    ? segments.length
    : Math.max(segments.length - 1, 0);

  for (let depth = 1; depth <= includeDepth; depth += 1) {
    const prefix = `${segments.slice(0, depth).join('/')}/`;
    if (prefix) targetSet.add(prefix);
  }
}

function validateUploadPath(value) {
  const raw = String(value || '').trim();

  if (!raw) {
    return { ok: true, value: '' };
  }

  if (raw.startsWith('/')) {
    return { ok: false, error: '上传路径不能以 / 开头' };
  }

  if (!raw.endsWith('/')) {
    return { ok: false, error: '上传路径必须以 / 结尾' };
  }

  if (raw.includes('\\')) {
    return { ok: false, error: '上传路径不能包含反斜杠 \\' };
  }

  if (raw.includes('//')) {
    return { ok: false, error: '上传路径不能包含连续的 /' };
  }

  const segments = raw.split('/').filter(Boolean);
  if (segments.some(segment => segment === '.' || segment === '..')) {
    return { ok: false, error: '上传路径不能包含 . 或 ..' };
  }

  if (segments.some(segment => segment.trim() !== segment || !segment)) {
    return { ok: false, error: '上传路径包含无效目录名' };
  }

  return { ok: true, value: raw };
}

async function validateOssAccess(config) {
  const client = new OSS({
    region: config.region,
    accessKeyId: config.accessKeyId,
    accessKeySecret: config.accessKeySecret,
    bucket: config.bucket,
  });

  await client.listV2({
    delimiter: '/',
    'max-keys': 1,
    'encoding-type': 'url',
  });
}

async function listAvailableBuckets(client) {
  const result = await client.listBuckets({
    'max-keys': 1000,
  });

  return (result.buckets || []).map(bucket => ({
    name: bucket.name,
    region: bucket.region,
  }));
}

function createBucketListClient(config) {
  return new OSS({
    region: config.region,
    accessKeyId: config.accessKeyId,
    accessKeySecret: config.accessKeySecret,
  });
}

function createApp() {
  const app = express();
  const upload = multer({ storage: multer.memoryStorage() });

  app.use(express.json());
  app.use(express.static(__dirname));

  // 上传接口：支持多文件
  app.post('/api/setup', async (req, res) => {
    const currentConfig = getEffectiveConfig();
    const nextConfig = {
      accessKeyId: req.body.accessKeyId || currentConfig.accessKeyId,
      accessKeySecret: req.body.accessKeySecret || currentConfig.accessKeySecret,
      bucket: req.body.bucket || currentConfig.bucket,
      region: req.body.region || currentConfig.region,
      prefix: req.body.prefix !== undefined ? req.body.prefix : currentConfig.prefix,
    };

    if (!nextConfig.accessKeyId || !nextConfig.accessKeySecret) {
      return res.status(400).json({ error: '请填写 AccessKey ID 和 AccessKey Secret' });
    }

    try {
      const shouldValidateAccess =
        nextConfig.accessKeyId !== currentConfig.accessKeyId
        || nextConfig.accessKeySecret !== currentConfig.accessKeySecret
        || nextConfig.bucket !== currentConfig.bucket
        || nextConfig.region !== currentConfig.region;

      if (shouldValidateAccess && nextConfig.bucket) {
        await validateOssAccess(nextConfig);
      }
      saveLocalConfig(nextConfig);
      cachedClient = null;
      cachedClientSignature = '';

      res.json({
        ok: true,
        config: getPublicConfig(),
      });
    } catch (err) {
      res.status(500).json({
        error: err.message || '保存本地配置失败',
      });
    }
  });

  app.post('/api/reset-config', (req, res) => {
    try {
      saveLocalConfig({
        accessKeyId: '',
        accessKeySecret: '',
        bucket: '',
        region: DEFAULT_REGION,
        prefix: '',
      });
      cachedClient = null;
      cachedClientSignature = '';
      res.json({
        ok: true,
        config: getPublicConfig(),
      });
    } catch (err) {
      res.status(500).json({
        error: err.message || '初始化应用失败',
      });
    }
  });

  app.post('/api/upload', upload.array('files'), async (req, res) => {
    const { config, client } = getRuntimeContext();

    if (!client) {
      return res.status(503).json({ error: '请先完成本地 OSS 配置' });
    }

    const pathCheck = validateUploadPath(req.body.path !== undefined ? req.body.path : config.prefix);
    if (!pathCheck.ok) {
      return res.status(400).json({ error: pathCheck.error });
    }

    const targetPrefix = pathCheck.value;

    if (!req.files || req.files.length === 0) {
      return res.status(400).json({ error: '没有文件' });
    }

    const results = [];

    for (const file of req.files) {
      const key = targetPrefix + file.originalname;
      try {
        const result = await client.put(key, file.buffer, {
          mime: file.mimetype,
        });
        results.push({
          name: file.originalname,
          key,
          prefix: targetPrefix,
          url: result.url,
          size: file.size,
          ok: true,
        });
      } catch (err) {
        results.push({
          name: file.originalname,
          key,
          prefix: targetPrefix,
          error: err.message,
          ok: false,
        });
      }
    }

    res.json({ results });
  });

  app.get('/api/prefixes', async (req, res) => {
    const { config, client } = getRuntimeContext();

    if (!client) {
      return res.status(503).json({ error: '请先完成本地 OSS 配置' });
    }

    try {
      const parentPrefix = normalizePrefix(req.query.parent || '');
      const prefixes = await listChildPrefixes(client, parentPrefix);
      res.json({
        prefixes,
        parentPrefix,
        defaultPrefix: config.prefix,
      });
    } catch (err) {
      res.status(500).json({
        error: err.message || '读取路径失败',
      });
    }
  });

  app.get('/api/prefixes/all', async (req, res) => {
    const { config, client } = getRuntimeContext();

    if (!client) {
      return res.status(503).json({ error: '请先完成本地 OSS 配置' });
    }

    try {
      const all = await listAllPrefixes(client);
      res.json({
        prefixes: all.prefixes,
        defaultPrefix: config.prefix,
        stats: all.stats,
      });
    } catch (err) {
      res.status(500).json({
        error: err.message || '读取完整路径失败',
      });
    }
  });

  app.get('/api/buckets', async (req, res) => {
    const config = getEffectiveConfig();
    if (!hasOssConfig(config)) {
      return res.status(503).json({ error: '请先填写 AccessKey ID 和 AccessKey Secret' });
    }

    try {
      const client = createBucketListClient(config);
      const buckets = await listAvailableBuckets(client);
      res.json({
        buckets,
        currentBucket: config.bucket,
        currentRegion: config.region,
      });
    } catch (err) {
      res.status(500).json({
        error: err.message || '读取 Bucket 列表失败',
      });
    }
  });

  app.post('/api/buckets/preview', async (req, res) => {
    const accessKeyId = String(req.body.accessKeyId || '').trim();
    const accessKeySecret = String(req.body.accessKeySecret || '').trim();
    const region = String(req.body.region || '').trim() || 'oss-cn-hongkong';

    if (!accessKeyId || !accessKeySecret) {
      return res.status(400).json({ error: '请先填写 AccessKey ID 和 AccessKey Secret' });
    }

    try {
      const client = createBucketListClient({
        accessKeyId,
        accessKeySecret,
        region,
      });
      const buckets = await listAvailableBuckets(client);
      res.json({ buckets, region });
    } catch (err) {
      res.status(500).json({
        error: err.message || '读取 Bucket 列表失败',
      });
    }
  });

  // 配置信息接口（不暴露 key，只暴露必要信息）
  app.get('/api/config', (req, res) => {
    res.json(getPublicConfig());
  });

  return app;
}

function startServer(options = {}) {
  const app = createApp();
  const host = options.host || process.env.HOST || '127.0.0.1';
  const port = Number(options.port || process.env.PORT || 3000);

  return new Promise((resolve, reject) => {
    const server = app.listen(port, host, () => {
      const config = getPublicConfig();

      console.log(`\n🚀 OSS 上传服务已启动`);
      console.log(`   地址: http://${host}:${port}`);
      console.log(`   Bucket: ${config.bucket || '未配置'}`);
      console.log(`   配置文件: ${config.configPath}\n`);
      if (!config.hasOssConfig) {
        console.log('   提示: 首次进入请先填写 AK/SK，保存后到主页面选择 Bucket\n');
      }
      resolve({ app, server, host, port, url: `http://${host}:${port}` });
    });

    server.on('error', reject);
  });
}

module.exports = {
  createApp,
  normalizePrefix,
  startServer,
};

if (require.main === module) {
  startServer().catch(err => {
    console.error(err);
    process.exit(1);
  });
}
