const appJson = require("./app.json");
const baseConfig = appJson.expo ?? {};

module.exports = ({ config = baseConfig } = {}) => {
  const experiments = {
    ...(baseConfig.experiments ?? {}),
    ...(config.experiments ?? {}),
  };

  return {
    ...baseConfig,
    ...config,
    experiments: {
      ...experiments,
      baseUrl: process.env.EXPO_BASE_URL ?? experiments.baseUrl ?? "",
    },
  };
};
