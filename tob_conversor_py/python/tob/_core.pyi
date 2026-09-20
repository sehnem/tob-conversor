class FrameStats:
    frames_read: int
    frames_accepted: int
    rejected_footer: int
    rejected_unconfirmed: int
    frame_base: int
    recovered: bool

    def __repr__(self) -> str: ...

class Header:
    station_name: str
    logger_model: str
    logger_sn: str
    logger_os: str
    logger_program: str
    table_name: str
    rec_intvl: float
    names: list[str]
    units: list[str]
    processing: list[str]
    dtypes: list[str]
    is_tob1: bool
    is_tob2: bool

    def __repr__(self) -> str: ...

def read_header(path: str) -> Header: ...
def scan_frames(path: str) -> FrameStats: ...
def to_parquet(input: str, output_dir: str, include_record: bool = ...) -> int: ...
def to_arrow_ipc(input: str, include_record: bool = ...) -> bytes: ...
def to_arrow_ipc_file_py(input: str, output: str, include_record: bool = ...) -> int: ...

class TobReader:
    def __init__(
        self,
        path: str,
        include_record: bool = ...,
        batch_size: int = ...,
    ) -> None: ...
    def __iter__(self) -> TobReader: ...
    def __next__(self) -> bytes: ...
    @property
    def frame_stats(self) -> FrameStats: ...
    @property
    def schema_ipc_bytes(self) -> bytes: ...
