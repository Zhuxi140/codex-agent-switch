UPDATE models
SET context_window = 258400,
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE context_window = 1050000
  AND model_id IN ('gpt-5.6', 'gpt-5.6-terra', 'gpt-5.6-luna')
  AND provider_id IN (
      SELECT id
      FROM providers
      WHERE preset_id = 'codex-native'
  );

UPDATE agent_thread_instances
SET context_window = 258400
WHERE context_window = 1050000
  AND agent_id IN (
      SELECT binding.agent_id
      FROM agent_model_bindings binding
      JOIN models model ON model.id = binding.model_id
      JOIN providers provider ON provider.id = model.provider_id
      WHERE provider.preset_id = 'codex-native'
        AND model.model_id IN ('gpt-5.6', 'gpt-5.6-terra', 'gpt-5.6-luna')
  );
