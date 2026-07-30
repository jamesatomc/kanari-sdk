enum KanariEnvironment {
  local('http://127.0.0.1:6767', 'http://127.0.0.1:3000'),
  dev('http://192.168.1.102:19001', 'http://192.168.1.101:3000');

  final String url;
  final String authUrl;

  const KanariEnvironment(this.url, this.authUrl);

  /// Returns the RPC endpoint URL
  String get rpcUrl => '$url/rpc';
}
