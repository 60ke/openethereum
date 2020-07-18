use std::{
    time::Instant,
    sync::{Mutex, Arc}
};

use crate::{
    rpc,
    rpc_apis
};

use hyper::{
    Method, Body, Request, Response, Server, StatusCode,
    service::{make_service_fn, service_fn}
};

use stats::{
	PrometheusMetrics,
	prometheus_gauge,
	prometheus::{self, Encoder}
};

#[derive(Debug, Clone, PartialEq)]
pub struct MetricsConfiguration {
	/// Are metrics enabled (default is false)?
	pub enabled: bool,
	/// The IP of the network interface used (default is 127.0.0.1).
	pub interface: String,
	/// The network port (default is 3000).
	pub port: u16,
}

impl Default for MetricsConfiguration {
	fn default() -> Self {
		MetricsConfiguration {
			enabled: false,
			interface: "127.0.0.1".into(),
			port: 3000,
		}
	}
}

struct State {
	rpc_apis: Arc<rpc_apis::FullDependencies>,
}

fn handle_request(req: Request<Body>, state: &Arc<Mutex<State>>) -> Response<Body> {
    let (parts, _body) = req.into_parts();
    match (parts.method, parts.uri.path()) {
        (Method::GET, "/metrics") => {
            let start = Instant::now();
            let mut reg = prometheus::Registry::new();

            let state = state.lock().unwrap();

            state.rpc_apis.client.prometheus_metrics(&mut reg);
            state.rpc_apis.sync.prometheus_metrics(&mut reg);

            let elapsed = start.elapsed();
            let ms = (elapsed.as_secs() as i64)*1000 + (elapsed.subsec_millis() as i64);
            prometheus_gauge(&mut reg, "metrics_time", "Time to perform rpc metrics", ms);

            let mut buffer = vec![];
            let encoder = prometheus::TextEncoder::new();
            let metric_families = reg.gather();

            encoder.encode(&metric_families, &mut buffer).expect("all source of metrics are static; qed");
            let text = String::from_utf8(buffer).expect("metrics encoding is ASCII; qed");

            Response::new(Body::from(text))
        },
        (_, _) => {
            let mut res = Response::new(Body::from("not found"));
            *res.status_mut() = StatusCode::NOT_FOUND;
            res
        }
    }
}

/// Start the prometheus metrics server accessible via GET <host>:<port>/metrics
pub fn start_prometheus_metrics(conf: &MetricsConfiguration, deps: &rpc::Dependencies<rpc_apis::FullDependencies>) -> Result<(), String> {

    if !conf.enabled {
        return Ok(());
    }

    let addr = format!("{}:{}", conf.interface, conf.port);
    let addr = addr
        .parse()
        .map_err(|err| format!("Failed to parse address '{}': {}", addr,err))?;

    let state = State {
        rpc_apis: deps.apis.clone(),
    };
    let state = Arc::new(Mutex::new(state));

    deps.executor.spawn_std( async move {
        let make_service = make_service_fn(move |_| {
            let state = state.clone();
            async move {
                Ok::<_, hyper::Error>(service_fn(move |req| {
                    let response = handle_request(req,&state);
                    async move { Ok::<_, hyper::Error>(response) }
                }))
            }
        });
        Server::bind(&addr).serve(make_service).await
            .expect("unable to create prometheus service.");
    });

    Ok(())
}
