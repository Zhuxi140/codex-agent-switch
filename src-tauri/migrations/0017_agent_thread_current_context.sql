ALTER TABLE agent_thread_instances
    ADD COLUMN current_context_tokens INTEGER CHECK (
        current_context_tokens IS NULL OR current_context_tokens >= 0
    );
