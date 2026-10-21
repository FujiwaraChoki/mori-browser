#include "chrome/browser/ui/views/mori/mori_favicons.h"

#include <memory>
#include <utility>

#include "base/functional/bind.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
#include "base/task/cancelable_task_tracker.h"
#include "chrome/browser/favicon/favicon_service_factory.h"
#include "chrome/browser/image_fetcher/image_fetcher_service_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/profiles/profile_key.h"
#include "components/favicon/content/content_favicon_driver.h"
#include "components/favicon/core/favicon_service.h"
#include "components/favicon_base/favicon_types.h"
#include "components/image_fetcher/core/image_fetcher.h"
#include "components/image_fetcher/core/image_fetcher_service.h"
#include "components/image_fetcher/core/request_metadata.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/navigation_entry.h"
#include "content/public/browser/web_contents.h"
#include "net/traffic_annotation/network_traffic_annotation.h"
#include "ui/gfx/image/image.h"
#include "url/gurl.h"

namespace mori {
namespace {

constexpr net::NetworkTrafficAnnotationTag kTrafficAnnotation =
    net::DefineNetworkTrafficAnnotation("mori_unloaded_tab_favicon", R"(
      semantics {
        sender: "Mori"
        description:
          "Fetches the favicon of a tab that hasn't loaded yet (one imported "
          "from another browser), so the sidebar can show it."
        trigger: "Importing tabs from another browser."
        data: "None; a request for the site's /favicon.ico."
        destination: WEBSITE
      }
      policy {
        cookies_allowed: NO
        setting: "None."
        policy_exception_justification: "Not implemented."
      })");

base::CancelableTaskTracker& Tracker() {
  static base::NoDestructor<base::CancelableTaskTracker> tracker;
  return *tracker;
}

// Shows what the database has for the page now.
void Refresh(base::WeakPtr<content::WebContents> contents) {
  if (!contents) {
    return;
  }
  if (auto* driver =
          favicon::ContentFaviconDriver::FromWebContents(contents.get())) {
    driver->FetchFavicon(contents->GetController().GetVisibleEntry()
                             ? contents->GetController()
                                   .GetVisibleEntry()
                                   ->GetVirtualURL()
                             : GURL(),
                         /*is_same_document=*/false);
  }
}

}  // namespace

void FetchIconForUnloadedTab(content::WebContents* contents) {
  content::NavigationEntry* entry =
      contents->GetController().GetVisibleEntry();
  const GURL page = entry ? entry->GetVirtualURL() : GURL();
  if (!page.SchemeIsHTTPOrHTTPS()) {
    return;
  }
  Profile* profile =
      Profile::FromBrowserContext(contents->GetBrowserContext());
  favicon::FaviconService* favicons = FaviconServiceFactory::GetForProfile(
      profile, ServiceAccessType::EXPLICIT_ACCESS);
  if (!favicons) {
    return;
  }
  base::WeakPtr<content::WebContents> weak = contents->GetWeakPtr();
  favicons->GetFaviconImageForPageURL(
      page,
      base::BindOnce(
          [](base::WeakPtr<content::WebContents> weak, Profile* profile,
             GURL page, const favicon_base::FaviconImageResult& result) {
            if (!weak) {
              return;
            }
            if (!result.image.IsEmpty()) {
              Refresh(weak);
              return;
            }
            // Not in the database: the site's /favicon.ico.
            image_fetcher::ImageFetcherService* service =
                ImageFetcherServiceFactory::GetForKey(
                    profile->GetProfileKey());
            if (!service) {
              return;
            }
            const GURL icon = page.GetWithEmptyPath().Resolve("/favicon.ico");
            service
                ->GetImageFetcher(
                    image_fetcher::ImageFetcherConfig::kNetworkOnly)
                ->FetchImage(
                    icon,
                    base::BindOnce(
                        [](base::WeakPtr<content::WebContents> weak,
                           Profile* profile, GURL page, GURL icon,
                           const gfx::Image& image,
                           const image_fetcher::RequestMetadata&) {
                          if (!weak || image.IsEmpty()) {
                            return;
                          }
                          favicon::FaviconService* favicons =
                              FaviconServiceFactory::GetForProfile(
                                  profile, ServiceAccessType::EXPLICIT_ACCESS);
                          if (!favicons) {
                            return;
                          }
                          favicons->SetOnDemandFavicons(
                              page, icon, favicon_base::IconType::kFavicon,
                              image,
                              base::BindOnce(
                                  [](base::WeakPtr<content::WebContents> weak,
                                     bool) { Refresh(weak); },
                                  weak));
                        },
                        weak, profile, page, icon),
                    image_fetcher::ImageFetcherParams(kTrafficAnnotation,
                                                      "MoriFavicon"));
          },
          weak, profile, page),
      &Tracker());
}

}  // namespace mori
