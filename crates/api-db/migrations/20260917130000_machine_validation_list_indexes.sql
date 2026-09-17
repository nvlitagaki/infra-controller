CREATE INDEX machine_validation_start_time_id_idx
    ON machine_validation (start_time DESC, id DESC);

CREATE INDEX machine_validation_machine_start_time_id_idx
    ON machine_validation (machine_id, start_time DESC, id DESC);

CREATE INDEX machine_validation_state_start_time_id_idx
    ON machine_validation (state, start_time DESC, id DESC);

CREATE INDEX machine_validation_machine_state_start_time_id_idx
    ON machine_validation (machine_id, state, start_time DESC, id DESC);
