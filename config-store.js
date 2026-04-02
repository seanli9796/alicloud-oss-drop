require('dotenv').config();
const fs = require('fs');
const path = require('path');

const DEFAULT_REGION = 'oss-cn-hongkong';

function normalizePrefix(value) {
  const normalized = String(value || '')
    .split('/')
    .map(segment => segment.trim())
    .filter(segment => segment && segment !== '.' && segment !== '..')
    .join('/');

  return normalized ? `${normalized}/` : '';
}

function normalizeRegion(value) {
  return String(value || '').trim() || DEFAULT_REGION;
}

function normalizeConfig(raw = {}) {
  return {
    accessKeyId: String(raw.accessKeyId || '').trim(),
    accessKeySecret: String(raw.accessKeySecret || '').trim(),
    bucket: String(raw.bucket || '').trim(),
    region: normalizeRegion(raw.region),
    prefix: normalizePrefix(raw.prefix),
  };
}

function getConfigFilePath() {
  return process.env.LOCAL_CONFIG_PATH || path.join(process.cwd(), '.local-config.json');
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf8'));
  } catch (err) {
    if (err.code === 'ENOENT') return {};
    throw err;
  }
}

function readLocalConfig() {
  return normalizeConfig(readJson(getConfigFilePath()));
}

function readEnvConfig() {
  return normalizeConfig({
    accessKeyId: process.env.OSS_ACCESS_KEY_ID,
    accessKeySecret: process.env.OSS_ACCESS_KEY_SECRET,
    bucket: process.env.OSS_BUCKET,
    region: process.env.OSS_REGION,
    prefix: process.env.OSS_PREFIX,
  });
}

function getEffectiveConfig() {
  const localRawConfig = readJson(getConfigFilePath());
  if (process.env.LOCAL_ONLY_CONFIG === 'true') {
    return normalizeConfig(localRawConfig);
  }

  const envConfig = readEnvConfig();

  return normalizeConfig({
    accessKeyId: localRawConfig.accessKeyId ?? envConfig.accessKeyId,
    accessKeySecret: localRawConfig.accessKeySecret ?? envConfig.accessKeySecret,
    bucket: localRawConfig.bucket ?? envConfig.bucket,
    region: localRawConfig.region ?? envConfig.region,
    prefix: localRawConfig.prefix ?? envConfig.prefix,
  });
}

function hasOssConfig(config = getEffectiveConfig()) {
  return Boolean(config.accessKeyId && config.accessKeySecret);
}

function hasBucketTarget(config = getEffectiveConfig()) {
  return Boolean(hasOssConfig(config) && config.bucket);
}

function saveLocalConfig(rawConfig) {
  const normalized = normalizeConfig(rawConfig);
  const filePath = getConfigFilePath();

  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(normalized, null, 2)}\n`, {
    mode: 0o600,
  });

  return normalized;
}

function getPublicConfig() {
  const config = getEffectiveConfig();

  return {
    bucket: config.bucket,
    region: config.region,
    prefix: config.prefix,
    hasOssConfig: hasOssConfig(config),
    configPath: getConfigFilePath(),
  };
}

module.exports = {
  DEFAULT_REGION,
  getConfigFilePath,
  getEffectiveConfig,
  getPublicConfig,
  hasOssConfig,
  hasBucketTarget,
  normalizeConfig,
  normalizePrefix,
  saveLocalConfig,
};
