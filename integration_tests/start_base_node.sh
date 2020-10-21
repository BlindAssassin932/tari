rm -rf temp/data/integration-test-base-node
mkdir -p  temp/data/integration-test-base-node
export TARI_BASE_NODE__NETWORK=localnet
export TARI_BASE_NODE__LOCALNET__DATA_DIR=localnet
export TARI_BASE_NODE__LOCALNET__DB_TYPE=lmdb
export TARI_BASE_NODE__LOCALNET__ORPHAN_STORAGE_CAPACITY=10
export TARI_BASE_NODE__LOCALNET__PRUNING_HORIZON=0
export TARI_BASE_NODE__LOCALNET__PRUNED_MODE_CLEANUP_INTERVAL=10000
export TARI_BASE_NODE__LOCALNET__CORE_THREADS=10
export TARI_BASE_NODE__LOCALNET__MAX_THREADS=512
export TARI_BASE_NODE__LOCALNET__IDENTITY_FILE=nodeid.json
export TARI_BASE_NODE__LOCALNET__TOR_IDENTITY_FILE=node_tor_id.json
export TARI_BASE_NODE__LOCALNET__WALLET_IDENTITY_FILE=walletid.json
export TARI_BASE_NODE__LOCALNET__WALLET_TOR_IDENTITY_FILE=wallet_tor_id.json
export TARI_BASE_NODE__LOCALNET__TRANSPORT=tcp
export TARI_BASE_NODE__LOCALNET__TCP_LISTENER_ADDRESS=/ip4/0.0.0.0/tcp/18189

export TARI_BASE_NODE__LOCALNET__PUBLIC_ADDRESS=/ip4/0.0.0.0/tcp/18189
#export TARI_BASE_NODE__LOCALNET__TOR_CONTROL_ADDRESS=/ip4/127.0.0.1/tcp/9051
#export TARI_BASE_NODE__LOCALNET__TOR_CONTROL_AUTH=none
#export TARI_BASE_NODE__LOCALNET__TOR_FORWARD_ADDRESS=/ip4/127.0.0.1/tcp/0
#export TARI_BASE_NODE__LOCALNET__TOR_ONION_PORT=18999
#export TARI_BASE_NODE__LOCALNET__PUBLIC_ADDRESS=
export TARI_BASE_NODE__LOCALNET__GRPC_ENABLED=true
export TARI_BASE_NODE__LOCALNET__GRPC_ADDRESS=127.0.0.1:50051
export TARI_BASE_NODE__LOCALNET__BLOCK_SYNC_STRATEGY=ViaBestChainMetadata
export TARI_BASE_NODE__LOCALNET__ENABLE_MINING=false
export TARI_BASE_NODE__LOCALNET__NUM_MINING_THREADS=1
# not used
export TARI_BASE_NODE__LOCALNET__GRPC_WALLET_ADDRESS=127.0.0.1:5999

export TARI_MERGE_MINING_PROXY__LOCALNET__MONEROD_URL=aasdf
export TARI_MERGE_MINING_PROXY__LOCALNET__MONEROD_USE_AUTH=false
export TARI_MERGE_MINING_PROXY__LOCALNET__MONEROD_USERNAME=asdf
export TARI_MERGE_MINING_PROXY__LOCALNET__MONEROD_PASSWORD=asdf
export TARI_MERGE_MINING_PROXY__LOCALNET__PROXY_HOST_ADDRESS=127.0.0.1:50071

cd temp/data/integration-test-base-node
cargo run --release --bin tari_base_node -- --base-path . --create-id --init
cargo run --release --bin tari_base_node -- --base-path .

