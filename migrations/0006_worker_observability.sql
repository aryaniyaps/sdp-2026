ALTER TABLE operations DROP CONSTRAINT operations_operation_type_check;
ALTER TABLE operations ADD CHECK (operation_type IN ('ingest','search','recall_v2','reflect_v2','worker_extract','worker_consolidate','worker_project','worker_rebuild','worker_clear_graph'));
