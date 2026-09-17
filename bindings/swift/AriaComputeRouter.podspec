Pod::Spec.new do |s|
  s.name                   = 'AriaComputeRouter'
  s.version                = ENV['ARIA_VERSION'] || '0.1.0'
  s.summary                = 'Swift binding for the Aria router (libaria-router_ffi FFI).'
  s.description            = 'High-level Swift API over the native libaria-router_ffi cdylib, ' \
                             'loaded at runtime via dlopen/dlsym.'
  s.homepage               = 'https://github.com/ariacompute/router'
  s.license                = { :type => 'MIT' }
  s.author                 = { 'AriaCompute' => 'https://github.com/ariacompute' }
  s.source                 = { :git => 'https://github.com/ariacompute/router.git', :tag => "v#{s.version}" }

  s.swift_version          = '5.9'
  s.ios.deployment_target  = '15.0'
  s.osx.deployment_target  = '13.0'

  # High-level Swift wrapper. Unlike the agent binding, this loads the native
  # library at runtime via dlopen, so there is no compiled FFI modulemap/header.
  s.source_files           = 'Sources/AriaComputeRouter/**/*'

  # The native lib is built separately (cargo build -p ariacompute-router-ffi)
  # and dropped in next to this podspec before `pod trunk push`. At runtime the
  # Router resolves it from ARIA_ROUTER_FFI_LIB or ~/.ariacompute/lib, so it is
  # vendored (not linked) and must be discoverable to the host app.
  s.vendored_libraries     = 'libaria_router_ffi.dylib'
  s.preserve_paths         = 'libaria_router_ffi.dylib'
  s.xcconfig               = {
    'OTHER_LDFLAGS' => '-L${PODS_ROOT}/AriaComputeRouter'
  }
end
