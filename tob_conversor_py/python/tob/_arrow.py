from __future__ import annotations

import io

import pyarrow as pa


def ipc_bytes_to_table(ipc_bytes: bytes) -> pa.Table:
    """Deserialize Arrow IPC File bytes to a PyArrow Table."""
    return pa.ipc.open_file(io.BytesIO(ipc_bytes)).read_all()


def ipc_stream_chunk_to_record_batch(chunk_bytes: bytes) -> pa.RecordBatch:
    """Deserialize a single IPC Stream chunk (schema + one batch) to a RecordBatch."""
    return pa.ipc.open_stream(io.BytesIO(chunk_bytes)).read_next_batch()
