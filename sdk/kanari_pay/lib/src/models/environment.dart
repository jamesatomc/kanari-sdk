enum KanariEnvironment {
  local('http://127.0.0.1:6767'),
  dev('http://192.168.1.101:19001');

  final String url;
  const KanariEnvironment(this.url);

  /// Returns the RPC endpoint URL
  String get rpcUrl {
    if (this == KanariEnvironment.dev) {
      return url; // Local server usually handles RPC at root or specific port
    }
    return '$url/rpc';
  }
}
