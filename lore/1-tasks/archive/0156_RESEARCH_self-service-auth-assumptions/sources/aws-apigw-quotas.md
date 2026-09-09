---
url: https://docs.aws.amazon.com/apigateway/latest/developerguide/limits.html
title: "Amazon API Gateway quotas (developer guide) + Service Quotas table from the AWS General Reference"
fetched_date: 2026-08-10
---

> This file archives two pages, both fetched 2026-08-10:
>
> 1. https://docs.aws.amazon.com/apigateway/latest/developerguide/limits.html
> 2. https://docs.aws.amazon.com/general/latest/gr/apigateway.html (Service quotas
>    section only; endpoint tables omitted as irrelevant)

---

# Part 1 — Amazon API Gateway quotas
<a name="limits"></a>

Source: https://docs.aws.amazon.com/apigateway/latest/developerguide/limits.html

The following quotas apply for all Amazon API Gateway API types.

## API Gateway account-level quotas, per Region
<a name="apigateway-account-level-limits-table"></a>

The following quotas apply per account, per Region in Amazon API Gateway.

| Resource or operation | Default quota | Can be increased |
| --- | --- | --- |
| Throttle quota per account, per Region across HTTP APIs, REST APIs, WebSocket APIs, and WebSocket callback APIs | 10,000 requests per second (RPS) with an additional burst capacity provided by the [token bucket algorithm](https://en.wikipedia.org/wiki/Token_bucket), using a maximum bucket capacity of 5,000 requests. \* The burst quota is determined by the API Gateway service team based on the overall RPS quota for the account in the Region. It is not a quota that a customer can control or request changes to. | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-8A5B8E43) |
| Throttle quota without access control per account per Region for a portal | 250,000 requests per second | No |
| Throttle quota with access control per account per Region for a portal | 10,000 requests per second | No |

\* For the following Regions, the default throttle quota is 2500 RPS and the default burst quota is 1250 RPS: Africa (Cape Town), Europe (Milan), Asia Pacific (Jakarta), Middle East (UAE), Asia Pacific (Hyderabad), Asia Pacific (Melbourne), Europe (Spain), Europe (Zurich), Israel (Tel Aviv), Canada West (Calgary), Asia Pacific (Malaysia), Asia Pacific (Thailand), and Mexico (Central).

## API Gateway quotas for creating, deploying and managing an API
<a name="api-gateway-control-service-limits-table"></a>

The following fixed quotas apply to creating, deploying, and managing an API in API Gateway, using the AWS CLI, the API Gateway console, or the API Gateway REST API and its SDKs. These quotas can't be increased.

| Action | Default quota | Can be increased |
| --- | --- | --- |
| CreateApiKey | 5 requests per second per account | No |
| CreateDeployment | 1 request every 5 seconds per account | No |
| CreateDocumentationVersion | 1 request every 20 seconds per account | No |
| CreateDomainName | 1 request every 30 seconds per account | No |
| CreateResource | 5 requests per second per account | No |
| CreateRestApi for Regional or private API | 1 request every 3 seconds per account | No |
| CreateRestApi for edge-optimized API | 1 request every 30 seconds per account | No |
| CreateVpcLink (V2) | 1 request every 15 seconds per account | No |
| DeleteApiKey | 5 requests per second per account | No |
| DeleteDomainName | 1 request every 30 seconds per account | No |
| DeleteResource | 5 requests per second per account | No |
| DeleteRestApi | 1 request every 30 seconds per account | No |
| GetResources | 5 requests every 2 seconds per account | No |
| DeleteVpcLink (V2) | 1 request every 30 seconds per account | No |
| ImportDocumentationParts | 1 request every 30 seconds per account | No |
| ImportRestApi for Regional or private API | 1 request every 3 seconds per account | No |
| ImportRestApi for edge-optimized API | 1 request every 30 seconds per account | No |
| PutRestApi | 1 request per second per account | No |
| UpdateAccount | 1 request every 20 seconds per account | No |
| UpdateDomainName | 1 request every 30 seconds per account | No |
| UpdateUsagePlan | 1 request every 20 seconds per account | No |
| Create Portal | 1 request every 3 seconds | No |
| Update Portal | 2 requests per minute | No |
| Get Portal | 10 requests per second | No |
| List Portals | 10 requests per second | No |
| Publish Portal | 2 requests per minute | No |
| DeletePortal | 2 requests per minute | No |
| PreviewPortal | 1 request every 3 seconds | No |
| DisablePortal | 2 requests per minute | No |
| GetPortalProduct | 10 requests per second | No |
| ListPortalProduct | 5 requests per second | No |
| CreatePortalProduct | 2 requests per second | No |
| UpdatePortalProduct | 0.5 requests per second | No |
| DeletePortalProduct | 1 request per second | No |
| PutPortalProductSharingPolicy | 1 request every 3 seconds | No |
| GetPortalProductSharingPolicy | 10 requests per second | No |
| DeletePortalProductSharingPolicy | 1 request every 3 seconds | No |
| CreateProductRestEndpointPage | 1 request every 3 seconds | No |
| GetProductRestEndpointPage | 10 requests per second | No |
| UpdateProductRestEndpointPage | 1 request every 3 seconds | No |
| DeleteProductRestEndpointPage | 1 request every 3 seconds | No |
| ListProductRestEndpointPages | 10 requests per second | No |
| CreateProductPage | 1 request every 3 seconds | No |
| GetProductPage | 10 requests per second | No |
| UpdateProductPage | 1 request every 3 seconds | No |
| DeleteProductPage | 1 request every 3 seconds | No |
| ListProductPages | 10 requests per second | No |
| Other operations | No quota up to the total account quota. | No |
| Total operations | 10 requests per second with a burst quota of 40 requests per second. | No |

---

# Part 2 — Amazon API Gateway endpoints and quotas (Service quotas)
<a name="limits_apigateway"></a>

Source: https://docs.aws.amazon.com/general/latest/gr/apigateway.html

Service quotas, also referred to as limits, are the maximum number of service resources or operations for your AWS account.

| Name | Default | Adjustable | Description |
| --- | --- | --- | --- |
| API Payload Size | Each supported Region: 10 Megabytes | No | Maximum payload size for non WebSocket API. |
| API Stage throttles in a usage plan | Each supported Region: 20 | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-A9DBC573) | The maximum number of API-stage throttle settings you can create in a usage plan in this account in the current region |
| API keys | Each supported Region: 10,000 | No | The maximum number of API keys that you can create in this account in the current region |
| AWS Lambda authorizer result size | Each supported Region: 8 Kilobytes | No | The maximum size of AWS Lambda authorizer result. |
| Client certificates | Each supported Region: 60 | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-824C9E42) | The maximum number of certificates that you can associate with this account in the current region |
| Connection duration for WebSocket API | Each supported Region: 7,200 Seconds | No | Maximum duration for WebSocket API connection. |
| Custom Domain Names | Each supported Region: 120 | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-A93447B8) | The maximum number of Custom domain names that you can create in this account in the current region |
| Domain name access associations | Each supported Region: 100 | Yes | The maximum number of domain name access associations that you can create in this account in the current region |
| Edge API URL Length | Each supported Region: 8,192 | No | Length, in characters, of the URL for an edge-optimized API. |
| Edge-optimized APIs | Each supported Region: 120 | No | The maximum number of edge-optimized APIs that you can create in this account in the current region |
| Maximum API caching TTL | Each supported Region: 3,600 Seconds | No | The maximum API caching TTL you can have in this account in the current region. |
| Maximum integration timeout in milliseconds | Each supported Region: 29,000 Milliseconds | Yes | The maximum integration timeout (in milliseconds) allowed for your account in the current region. This limit can only be increased for Regional and private APIs. |
| Maximum resource policy size in bytes | Each supported Region: 8,192 | Yes | The maximum resource policy size in bytes you can have in this account in the current region. |
| Method ARN Length | Each supported Region: 1,600 Bytes | No | ARN length of a method with authorization |
| Private APIs | Each supported Region: 600 | No | The maximum number of private APIs that you can create in this account in the current region |
| Regional API URL Length | Each supported Region: 10,240 | No | Length, in characters, of the URL for a regional API |
| Regional APIs | Each supported Region: 600 | No | The maximum number of regional APIs that you can create in this account in the current region |
| Resources/Routes per REST/WebSocket API | Each supported Region: 300 | Yes | The maximum number of resources/routes that you can include in a REST or WebSocket API |
| Routes per HTTP API | Each supported Region: 300 | Yes | The maximum number of routes that you can include in an HTTP API |
| Stage variables per stage | Each supported Region: 100 | No | Stage variables per stage |
| Stages per API | Each supported Region: 10 | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-379E48B0) | The maximum number of stages that you can create for an API |
| Tags Per Stage | Each supported Region: 50 | No | Maximum tags per stage. |
| Throttle burst rate | af-south-1: 1,250<br />ap-east-2: 1,250<br />ap-south-2: 1,250<br />ap-southeast-3: 1,250<br />ap-southeast-4: 1,250<br />ap-southeast-5: 1,250<br />ap-southeast-6: 1,250<br />ap-southeast-7: 1,250<br />ca-west-1: 1,250<br />eu-central-2: 1,250<br />eu-south-1: 1,250<br />eu-south-2: 1,250<br />il-central-1: 1,250<br />mx-central-1: 1,250<br />Each of the other supported Regions: 5,000 | No | The maximum number of additional requests per second (RPS) that you can send in one burst in this account in the current region |
| Throttle rate | af-south-1: 2,500<br />ap-east-2: 2,500<br />ap-south-2: 2,500<br />ap-southeast-3: 2,500<br />ap-southeast-4: 2,500<br />ap-southeast-5: 2,500<br />ap-southeast-6: 2,500<br />ap-southeast-7: 2,500<br />ca-west-1: 2,500<br />eu-central-2: 2,500<br />eu-south-1: 2,500<br />eu-south-2: 2,500<br />il-central-1: 2,500<br />mx-central-1: 2,500<br />Each of the other supported Regions: 10,000 | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-8A5B8E43) | The maximum number of requests per second that your APIs can receive in this account in the current region |
| Usage plans | Each supported Region: 300 | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-E8693075) | The maximum number of usage plans that you can create in this account in the current region |
| Usage plans per API key | Each supported Region: 10 | [Yes](https://console.aws.amazon.com/servicequotas/home/services/apigateway/quotas/L-985EB478) | The maximum number of usage plans that you can associate with an API key |
| VPC links | Each supported Region: 20 | Yes | The maximum number of VPC links that you can create in this account in the current region |
| VPC links(V2) | Each supported Region: 10 | Yes | The maximum number of V2 VPC links that you can create in this account in the current Region |
| WebSocket Idle Connection Timeout | Each supported Region: 600 Seconds | No | WebSocket API idle connection timeout. |
| WebSocket frame size | Each supported Region: 32 Kilobytes | No | Maximum WebSocket frame size. |
| WebSocket message payload size | Each supported Region: 128 Kilobytes | No | Maximum WebSocket message payload size. |
| WebSocket new connections burst rate | Each supported Region: 500 | No | New connections in burst capacity per account (across all WebSocket APIs) per region |
| WebSocket new connections rate | Each supported Region: 500 | Yes | New connections per second per account (across all WebSocket APIs) per region |

> Portal-related rows (Portals per account, PortalProducts, ProductPages, Maximum
> Logo Size, etc.) omitted from this archive as out of scope; see the live page.
